//! Streaming UTF-8 character windows for tool file reads.
//!
//! Reads only the requested window into memory: skips `offset` chars, then
//! collects up to `max_chars`. Does not load the whole file.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

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
/// # Errors
///
/// Returns I/O errors from opening/reading the file, or `InvalidData` if the
/// file is not valid UTF-8.
pub fn read_char_window(path: &Path, offset: usize, max_chars: usize) -> io::Result<ReadWindow> {
    let max_chars = max_chars.max(1);
    let file = File::open(path)?;
    let mut reader = io::BufReader::with_capacity(64 * 1024, file);

    let mut pending = Vec::new();
    let mut byte_buf = [0u8; 8 * 1024];
    let mut skip_remaining = offset;
    let mut content = String::new();
    let mut chars_returned = 0usize;
    let mut truncated = false;

    loop {
        let n = reader.read(&mut byte_buf)?;
        if n == 0 {
            if !pending.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid utf-8: incomplete sequence at end of file",
                ));
            }
            break;
        }

        pending.extend_from_slice(&byte_buf[..n]);
        let decoded = decode_pending(&pending)?;
        if !decoded.text.is_empty() {
            let outcome = consume_chars(
                decoded.text,
                &mut skip_remaining,
                max_chars,
                &mut content,
                &mut chars_returned,
            );
            pending.drain(..decoded.drain_to);
            match outcome {
                ConsumeOutcome::NeedMoreInput => {}
                ConsumeOutcome::WindowFull { more_in_chunk } => {
                    truncated =
                        more_in_chunk || has_more_chars(&mut reader, &mut pending, &mut byte_buf)?;
                    break;
                }
            }
        } else if decoded.drain_to > 0 {
            pending.drain(..decoded.drain_to);
        }
    }

    let next_offset = if truncated {
        Some(offset.saturating_add(chars_returned))
    } else {
        None
    };

    Ok(ReadWindow {
        content,
        offset,
        chars_returned,
        truncated,
        next_offset,
    })
}

struct Decoded<'a> {
    text: &'a str,
    drain_to: usize,
}

fn decode_pending(pending: &[u8]) -> io::Result<Decoded<'_>> {
    match std::str::from_utf8(pending) {
        Ok(text) => Ok(Decoded {
            text,
            drain_to: pending.len(),
        }),
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            if error.error_len().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid utf-8 in file",
                ));
            }
            // Incomplete trailing sequence — keep bytes after valid_up_to.
            if valid_up_to == 0 {
                return Ok(Decoded {
                    text: "",
                    drain_to: 0,
                });
            }
            let text = std::str::from_utf8(&pending[..valid_up_to])
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            Ok(Decoded {
                text,
                drain_to: valid_up_to,
            })
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
    for ch in text.chars() {
        if *skip_remaining > 0 {
            *skip_remaining = skip_remaining.saturating_sub(1);
            continue;
        }
        if *chars_returned >= max_chars {
            return ConsumeOutcome::WindowFull {
                more_in_chunk: true,
            };
        }
        content.push(ch);
        *chars_returned = chars_returned.saturating_add(1);
    }
    if *chars_returned >= max_chars {
        ConsumeOutcome::WindowFull {
            more_in_chunk: false,
        }
    } else {
        ConsumeOutcome::NeedMoreInput
    }
}

fn has_more_chars(
    reader: &mut io::BufReader<File>,
    pending: &mut Vec<u8>,
    byte_buf: &mut [u8],
) -> io::Result<bool> {
    loop {
        if !pending.is_empty() {
            let decoded = decode_pending(pending)?;
            if !decoded.text.is_empty() {
                return Ok(true);
            }
            if decoded.drain_to > 0 {
                pending.drain(..decoded.drain_to);
            }
            // Incomplete sequence only — need more bytes.
        }

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
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        file.write_all(contents.as_bytes()).expect("write");
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
}
