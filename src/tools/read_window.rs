//! Streaming UTF-8 character windows for tool file reads.
//!
//! Reads only the requested window into memory: skips `offset` chars, then
//! collects up to `max_chars`. Does not load the whole file.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

const DECODE_CHUNK: usize = 8 * 1024;
/// Max bytes of an incomplete UTF-8 sequence kept between chunks.
const UTF8_MAX_INCOMPLETE: usize = 4;

/// One window of text from a file, addressed by UTF-8 character offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadWindow {
    pub content: String,
    pub offset: usize,
    pub chars_returned: usize,
    /// True when the window ended before EOF (more characters remain).
    pub truncated: bool,
    /// When [`Self::truncated`], the character offset to pass on the next read.
    pub next_offset: Option<usize>,
}

/// Reads a UTF-8 character window from `path`.
///
/// `max_chars` of 0 is treated as 1.
///
/// # Errors
///
/// Returns I/O errors from opening/reading the file, or `InvalidData` if the
/// requested window cannot be decoded as UTF-8 (a definite invalid sequence, or
/// an incomplete sequence at EOF, while more characters are still required). A
/// valid prefix that fills the window succeeds even if later bytes are invalid;
/// `truncated` is then true so the next offset can surface the error.
pub fn read_char_window(path: &Path, offset: usize, max_chars: usize) -> io::Result<ReadWindow> {
    let max_chars = max_chars.max(1);
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    let mut pending = Vec::with_capacity(DECODE_CHUNK + UTF8_MAX_INCOMPLETE);
    let mut pending_start = 0usize;
    let mut byte_buf = [0u8; DECODE_CHUNK];
    let mut skip_remaining = offset;
    let mut content = String::with_capacity(max_chars);
    let mut chars_returned = 0usize;
    let mut truncated = false;

    loop {
        compact_pending(&mut pending, &mut pending_start);
        let n = reader.read(&mut byte_buf)?;
        if n == 0 {
            if pending_start < pending.len() {
                return Err(invalid_utf8(
                    "invalid utf-8: incomplete sequence at end of file",
                ));
            }
            break;
        }

        pending.extend_from_slice(&byte_buf[..n]);
        let decoded = decode_pending(&pending[pending_start..]);
        if !decoded.text.is_empty() {
            let outcome = consume_chars(
                decoded.text,
                &mut skip_remaining,
                max_chars,
                &mut content,
                &mut chars_returned,
            );
            pending_start += decoded.drain_to;
            match outcome {
                ConsumeOutcome::NeedMoreInput => {
                    if decoded.invalid {
                        return Err(invalid_utf8("invalid utf-8 in file"));
                    }
                }
                ConsumeOutcome::WindowFull { more_in_chunk } => {
                    truncated = more_in_chunk
                        || decoded.invalid
                        || has_more_chars(
                            &mut reader,
                            &mut pending,
                            &mut pending_start,
                            &mut byte_buf,
                        )?;
                    break;
                }
            }
        } else if decoded.invalid {
            return Err(invalid_utf8("invalid utf-8 in file"));
        } else if decoded.drain_to > 0 {
            pending_start += decoded.drain_to;
        }
    }

    let next_offset = if truncated {
        Some(offset.saturating_add(chars_returned))
    } else {
        None
    };
    debug_assert_eq!(truncated, next_offset.is_some());

    Ok(ReadWindow {
        content,
        offset,
        chars_returned,
        truncated,
        next_offset,
    })
}

fn invalid_utf8(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn compact_pending(pending: &mut Vec<u8>, start: &mut usize) {
    if *start == 0 {
        return;
    }
    let len = pending.len();
    if *start >= len {
        pending.clear();
    } else {
        pending.copy_within(*start.., 0);
        pending.truncate(len - *start);
    }
    *start = 0;
}

struct Decoded<'a> {
    text: &'a str,
    drain_to: usize,
    /// Definite invalid UTF-8 follows `text` (not merely an incomplete sequence).
    invalid: bool,
}

fn decode_pending(pending: &[u8]) -> Decoded<'_> {
    match std::str::from_utf8(pending) {
        Ok(text) => Decoded {
            text,
            drain_to: pending.len(),
            invalid: false,
        },
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            let text = if valid_up_to == 0 {
                ""
            } else {
                std::str::from_utf8(&pending[..valid_up_to])
                    .expect("BUG: Utf8Error::valid_up_to prefix is valid UTF-8")
            };
            Decoded {
                text,
                drain_to: valid_up_to,
                invalid: error.error_len().is_some(),
            }
        }
    }
}

enum ConsumeOutcome {
    NeedMoreInput,
    WindowFull { more_in_chunk: bool },
}

fn consume_chars(
    text: &str,
    skip_remaining: &mut usize,
    max_chars: usize,
    content: &mut String,
    chars_returned: &mut usize,
) -> ConsumeOutcome {
    let mut rest = text;

    if *skip_remaining > 0 {
        let (_, leftover, skipped_count) = split_chars(rest, *skip_remaining);
        *skip_remaining = skip_remaining.saturating_sub(skipped_count);
        rest = leftover;
        if *skip_remaining > 0 {
            return ConsumeOutcome::NeedMoreInput;
        }
    }

    let need = max_chars.saturating_sub(*chars_returned);
    if need == 0 {
        return ConsumeOutcome::WindowFull {
            more_in_chunk: !rest.is_empty(),
        };
    }

    let (taken, leftover, taken_count) = split_chars(rest, need);
    content.push_str(taken);
    *chars_returned = chars_returned.saturating_add(taken_count);
    if *chars_returned >= max_chars {
        ConsumeOutcome::WindowFull {
            more_in_chunk: !leftover.is_empty(),
        }
    } else {
        ConsumeOutcome::NeedMoreInput
    }
}

fn split_chars(text: &str, max_chars: usize) -> (&str, &str, usize) {
    if max_chars == 0 {
        return ("", text, 0);
    }
    let mut count = 0usize;
    for (idx, _) in text.char_indices() {
        if count == max_chars {
            return (&text[..idx], &text[idx..], count);
        }
        count += 1;
    }
    (text, "", count)
}

fn has_more_chars(
    reader: &mut BufReader<File>,
    pending: &mut Vec<u8>,
    pending_start: &mut usize,
    byte_buf: &mut [u8],
) -> io::Result<bool> {
    loop {
        if *pending_start < pending.len() {
            let decoded = decode_pending(&pending[*pending_start..]);
            if decoded.invalid || !decoded.text.is_empty() {
                return Ok(true);
            }
            if decoded.drain_to > 0 {
                *pending_start += decoded.drain_to;
            }
        }

        compact_pending(pending, pending_start);
        let n = reader.read(byte_buf)?;
        if n == 0 {
            return Ok(false);
        }
        pending.extend_from_slice(&byte_buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(contents: &str) -> tempfile::NamedTempFile {
        write_temp_bytes(contents.as_bytes())
    }

    fn write_temp_bytes(contents: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        file.write_all(contents).expect("write");
        file.flush().expect("flush");
        file
    }

    #[test]
    fn reads_full_small_file() {
        let file = write_temp("hello");
        let window = read_char_window(file.path(), 0, 100).expect("read");
        assert_eq!(window.content, "hello");
        assert!(!window.truncated);
        assert_eq!(window.chars_returned, 5);
        assert_eq!(window.next_offset, None);
    }

    #[test]
    fn reads_empty_file() {
        let file = write_temp("");
        let window = read_char_window(file.path(), 0, 50).expect("read");
        assert_eq!(window.content, "");
        assert!(!window.truncated);
        assert_eq!(window.chars_returned, 0);
        assert_eq!(window.next_offset, None);
    }

    #[test]
    fn zero_max_chars_is_treated_as_one() {
        let file = write_temp("ab");
        let window = read_char_window(file.path(), 0, 0).expect("read");
        assert_eq!(window.content, "a");
        assert_eq!(window.chars_returned, 1);
        assert!(window.truncated);
        assert_eq!(window.next_offset, Some(1));
    }

    #[test]
    fn windows_with_offset() {
        let file = write_temp("abcdefghijklmnopqrstuvwxyz");
        let first = read_char_window(file.path(), 0, 10).expect("first");
        assert_eq!(first.content, "abcdefghij");
        assert!(first.truncated);
        assert_eq!(first.next_offset, Some(10));

        let second = read_char_window(file.path(), 10, 10).expect("second");
        assert_eq!(second.content, "klmnopqrst");
        assert!(second.truncated);
        assert_eq!(second.next_offset, Some(20));

        let third = read_char_window(file.path(), 20, 10).expect("third");
        assert_eq!(third.content, "uvwxyz");
        assert!(!third.truncated);
        assert_eq!(third.next_offset, None);
    }

    #[test]
    fn handles_multibyte_chars() {
        let file = write_temp("🙂🙂🙂🙂");
        let window = read_char_window(file.path(), 1, 2).expect("read");
        assert_eq!(window.content, "🙂🙂");
        assert!(window.truncated);
        assert_eq!(window.chars_returned, 2);
        assert_eq!(window.next_offset, Some(3));
    }

    #[test]
    fn offset_past_eof_returns_empty() {
        let file = write_temp("abc");
        let window = read_char_window(file.path(), 10, 50).expect("read");
        assert_eq!(window.content, "");
        assert!(!window.truncated);
        assert_eq!(window.chars_returned, 0);
        assert_eq!(window.next_offset, None);
    }

    #[test]
    fn exact_window_at_eof_is_not_truncated() {
        let file = write_temp("12345");
        let window = read_char_window(file.path(), 0, 5).expect("read");
        assert_eq!(window.content, "12345");
        assert!(!window.truncated);
        assert_eq!(window.next_offset, None);
    }

    #[test]
    fn valid_prefix_window_succeeds_when_invalid_bytes_follow() {
        let mut bytes = b"hello".to_vec();
        bytes.push(0xFF);
        let file = write_temp_bytes(&bytes);

        let window = read_char_window(file.path(), 0, 5).expect("valid window");
        assert_eq!(window.content, "hello");
        assert!(window.truncated);
        assert_eq!(window.next_offset, Some(5));

        let error = read_char_window(file.path(), 5, 5).expect_err("next window hits invalid");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn invalid_utf8_errors_when_window_still_needs_chars() {
        let mut bytes = b"hello".to_vec();
        bytes.push(0xFF);
        let file = write_temp_bytes(&bytes);
        let error = read_char_window(file.path(), 0, 100).expect_err("needs chars past invalid");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn incomplete_sequence_at_eof_errors_while_collecting() {
        let file = write_temp_bytes(&[b'a', b'b', 0xC3]);
        let error = read_char_window(file.path(), 0, 10).expect_err("incomplete");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn incomplete_trailing_bytes_after_full_window_are_not_more_chars() {
        let mut bytes = b"hello".to_vec();
        bytes.push(0xC3);
        let file = write_temp_bytes(&bytes);
        let window = read_char_window(file.path(), 0, 5).expect("full window");
        assert_eq!(window.content, "hello");
        assert!(!window.truncated);
        assert_eq!(window.next_offset, None);
    }

    #[test]
    fn multibyte_char_split_across_decode_chunk() {
        let mut bytes = vec![b'a'; DECODE_CHUNK - 1];
        bytes.extend_from_slice("é".as_bytes());
        let file = write_temp_bytes(&bytes);
        let window = read_char_window(file.path(), DECODE_CHUNK - 1, 1).expect("split char");
        assert_eq!(window.content, "é");
        assert_eq!(window.chars_returned, 1);
        assert!(!window.truncated);
    }
}
