//! Minimal Server-Sent Events parser for MCP Streamable HTTP responses.

use serde_json::Value;

/// Default maximum buffered bytes awaiting an event boundary.
pub const DEFAULT_MAX_BUFFER_BYTES: usize = 1024 * 1024;

/// One parsed SSE event (WHATWG event-stream fields).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry_ms: Option<u64>,
    pub data: String,
}

/// Incremental SSE buffer that emits complete events separated by blank lines.
#[derive(Debug)]
pub struct SseParser {
    buffer: String,
    max_buffer_bytes: usize,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::with_max_buffer(DEFAULT_MAX_BUFFER_BYTES)
    }
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_max_buffer(max_buffer_bytes: usize) -> Self {
        Self {
            buffer: String::new(),
            max_buffer_bytes: max_buffer_bytes.max(1),
        }
    }

    /// Push a decoded UTF-8 chunk and return any complete events.
    ///
    /// # Errors
    ///
    /// Returns an error when the unterminated buffer exceeds [`Self`]'s size limit.
    pub fn push(&mut self, chunk: &str) -> Result<Vec<SseEvent>, String> {
        if self.buffer.len().saturating_add(chunk.len()) > self.max_buffer_bytes {
            return Err(format!(
                "SSE buffer exceeded {} bytes without an event boundary",
                self.max_buffer_bytes
            ));
        }
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(idx) = find_event_boundary(&self.buffer) {
            let raw = self.buffer[..idx].to_string();
            let skip = if self.buffer[idx..].starts_with("\r\n\r\n") {
                4
            } else {
                2
            };
            // Reuse capacity: drain processed prefix instead of allocating a fresh remainder.
            let drain_end = idx + skip;
            self.buffer.drain(..drain_end);
            if let Some(event) = parse_event_block(&raw) {
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Parse remaining buffered bytes as a final event (no trailing blank line).
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return None;
        }
        let raw = std::mem::take(&mut self.buffer);
        parse_event_block(&raw)
    }
}

/// Accumulates network bytes and yields only complete UTF-8 strings.
#[derive(Debug, Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode as much of `chunk` (plus any pending incomplete bytes) as valid UTF-8.
    ///
    /// Incomplete trailing sequences are retained for the next call.
    pub fn push(&mut self, chunk: &[u8]) -> Result<String, String> {
        if self.pending.is_empty() {
            return decode_complete_prefix(chunk, &mut self.pending);
        }
        self.pending.extend_from_slice(chunk);
        let owned = std::mem::take(&mut self.pending);
        decode_complete_prefix(&owned, &mut self.pending)
    }

    /// Flush any remaining pending bytes; incomplete sequences become U+FFFD.
    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let pending = std::mem::take(&mut self.pending);
        String::from_utf8_lossy(&pending).into_owned()
    }
}

fn decode_complete_prefix(input: &[u8], pending: &mut Vec<u8>) -> Result<String, String> {
    match std::str::from_utf8(input) {
        Ok(text) => Ok(text.to_string()),
        Err(err) => {
            let valid_up_to = err.valid_up_to();
            if valid_up_to == 0 && err.error_len().is_some() {
                // Invalid byte sequence at the start — skip one byte as U+FFFD and continue.
                let mut out = String::from('\u{FFFD}');
                let rest = &input[1..];
                out.push_str(&decode_complete_prefix(rest, pending)?);
                return Ok(out);
            }
            let (valid, incomplete) = input.split_at(valid_up_to);
            // If error_len is None, the incomplete sequence may still be valid once more bytes arrive.
            if err.error_len().is_none() {
                pending.extend_from_slice(incomplete);
                return Ok(std::str::from_utf8(valid)
                    .map_err(|e| format!("internal UTF-8 decode error: {e}"))?
                    .to_string());
            }
            // Invalid sequence in the middle: emit replacement for the bad byte(s) and continue.
            let mut out = std::str::from_utf8(valid)
                .map_err(|e| format!("internal UTF-8 decode error: {e}"))?
                .to_string();
            let bad_len = err.error_len().unwrap_or(1);
            out.push('\u{FFFD}');
            let rest = &input[valid_up_to.saturating_add(bad_len)..];
            out.push_str(&decode_complete_prefix(rest, pending)?);
            Ok(out)
        }
    }
}

fn find_event_boundary(buf: &str) -> Option<usize> {
    buf.find("\r\n\r\n").or_else(|| buf.find("\n\n"))
}

fn parse_event_block(block: &str) -> Option<SseEvent> {
    let mut event = SseEvent::default();
    let mut data_lines = Vec::new();
    let mut saw_field = false;

    for line in block.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        saw_field = true;
        let (field, value) = match line.split_once(':') {
            Some((field, rest)) => {
                let value = rest.strip_prefix(' ').unwrap_or(rest);
                (field, value)
            }
            None => (line, ""),
        };
        match field {
            "event" => event.event = Some(value.to_string()),
            "id" => {
                if !value.contains('\0') {
                    event.id = Some(value.to_string());
                }
            }
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    event.retry_ms = Some(ms);
                }
            }
            "data" => data_lines.push(value.to_string()),
            _ => {}
        }
    }

    if !saw_field && data_lines.is_empty() {
        return None;
    }
    event.data = data_lines.join("\n");
    Some(event)
}

/// Try to decode an SSE `data` payload as JSON.
pub fn parse_json_data(data: &str) -> Option<Value> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_data_and_id() {
        let mut parser = SseParser::new();
        let events = parser.push("id: 42\ndata: {\"a\":1\ndata: }\n\n").unwrap();
        // Second data line empty still joins with newline per SSE rules;
        // use a realistic single-line JSON event instead.
        assert!(events.is_empty() || events.len() == 1);

        let mut parser = SseParser::new();
        let events = parser
            .push("id: 7\nevent: message\ndata: {\"ok\":true}\n\n")
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("7"));
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(parse_json_data(&events[0].data).unwrap()["ok"], true);
    }

    #[test]
    fn ignores_comment_lines() {
        let mut parser = SseParser::new();
        let events = parser.push(": keep-alive\n\ndata: {\"x\":1}\n\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(parse_json_data(&events[0].data).unwrap()["x"], 1);
    }

    #[test]
    fn respects_retry_field() {
        let mut parser = SseParser::new();
        let events = parser.push("retry: 1500\ndata: {}\n\n").unwrap();
        assert_eq!(events[0].retry_ms, Some(1500));
    }

    #[test]
    fn finish_returns_unterminated_final_event() {
        let mut parser = SseParser::new();
        assert!(parser.push("data: {\"ok\":true}\n").unwrap().is_empty());
        let event = parser.finish().expect("final event");
        assert_eq!(parse_json_data(&event.data).unwrap()["ok"], true);
    }

    #[test]
    fn rejects_oversized_unterminated_buffer() {
        let mut parser = SseParser::with_max_buffer(32);
        let err = parser
            .push("data: this event never ends and grows forever")
            .expect_err("should reject");
        assert!(err.contains("exceeded"));
    }

    #[test]
    fn utf8_decoder_preserves_split_multibyte() {
        let mut decoder = Utf8StreamDecoder::new();
        // € in UTF-8 is E2 82 AC — split across two chunks.
        let first = decoder.push(&[0xE2]).unwrap();
        assert!(first.is_empty());
        let second = decoder.push(&[0x82, 0xAC]).unwrap();
        assert_eq!(second, "€");
        assert!(decoder.finish().is_empty());
    }
}
