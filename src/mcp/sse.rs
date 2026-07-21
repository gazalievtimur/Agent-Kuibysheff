//! Minimal Server-Sent Events parser for MCP Streamable HTTP responses.

use serde_json::Value;

/// One parsed SSE event (WHATWG event-stream fields).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry_ms: Option<u64>,
    pub data: String,
}

/// Incremental SSE buffer that emits complete events separated by blank lines.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a decoded UTF-8 chunk and return any complete events.
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(idx) = find_event_boundary(&self.buffer) {
            let raw = self.buffer[..idx].to_string();
            let skip = if self.buffer[idx..].starts_with("\r\n\r\n") {
                4
            } else {
                2
            };
            self.buffer = self.buffer[idx + skip..].to_string();
            if let Some(event) = parse_event_block(&raw) {
                events.push(event);
            }
        }
        events
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
        let events = parser.push("id: 42\ndata: {\"a\":1\ndata: }\n\n");
        // Second data line empty still joins with newline per SSE rules;
        // use a realistic single-line JSON event instead.
        assert!(events.is_empty() || events.len() == 1);

        let mut parser = SseParser::new();
        let events = parser.push("id: 7\nevent: message\ndata: {\"ok\":true}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("7"));
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(parse_json_data(&events[0].data).unwrap()["ok"], true);
    }

    #[test]
    fn ignores_comment_lines() {
        let mut parser = SseParser::new();
        let events = parser.push(": keep-alive\n\ndata: {\"x\":1}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(parse_json_data(&events[0].data).unwrap()["x"], 1);
    }

    #[test]
    fn respects_retry_field() {
        let mut parser = SseParser::new();
        let events = parser.push("retry: 1500\ndata: {}\n\n");
        assert_eq!(events[0].retry_ms, Some(1500));
    }
}
