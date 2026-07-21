//! Bounded stdout/stderr collectors: truncate for callers, keep draining the pipe.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Truncates UTF-8 text to `max_chars`, returning whether truncation occurred.
#[must_use]
pub fn truncate_utf8_chars(text: &str, max_chars: usize) -> (String, bool) {
    let total = text.chars().count();
    if total > max_chars {
        (text.chars().take(max_chars).collect(), true)
    } else {
        (text.to_owned(), false)
    }
}

/// Reads `reader` to EOF, keeping at most `max_chars` UTF-8 characters for the returned string.
///
/// Bytes beyond the limit are discarded, but the pipe continues to be drained so producers
/// cannot deadlock on a full buffer.
///
/// # Errors
///
/// Returns I/O errors from the underlying reader.
pub async fn collect_utf8_bounded<R>(mut reader: R, max_chars: usize) -> io::Result<(String, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8_192];

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if truncated {
            continue;
        }

        kept.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&kept);
        let count = text.chars().count();
        if count > max_chars {
            let trimmed: String = text.chars().take(max_chars).collect();
            kept = trimmed.into_bytes();
            truncated = true;
        }
    }

    let text = String::from_utf8_lossy(&kept).into_owned();
    let (text, also_truncated) = truncate_utf8_chars(&text, max_chars);
    Ok((text, truncated || also_truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn collect_truncates_but_drains_remainder() {
        let payload = "abcdefghij";
        let (text, truncated) = collect_utf8_bounded(Cursor::new(payload), 4)
            .await
            .expect("collect");
        assert_eq!(text, "abcd");
        assert!(truncated);
    }

    #[test]
    fn truncate_utf8_chars_handles_multibyte() {
        let (text, truncated) = truncate_utf8_chars("яяяя", 2);
        assert_eq!(text, "яя");
        assert!(truncated);
    }
}
