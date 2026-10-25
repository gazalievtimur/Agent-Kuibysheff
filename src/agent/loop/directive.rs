use serde::Deserialize;
use serde_json::Value;

pub(crate) const FILE_BEGIN: &str = "<<<FILE";
pub(crate) const FILE_END: &str = "FILE>>>";

const GENERAL_HINT: &str = "Respond with exactly one JSON object. Previous turn was not executed. Do not repeat the rejected body. Small files use arguments.content. For BSL, XML, or any text with quotes or longer than a few hundred characters, omit content and set arguments.body to \"raw\". After the JSON object write a newline, <<<FILE, a newline, the file bytes, a newline, and FILE>>>. If that body was cut off, resend only the remaining text with mode \"append\" and a complete <<<FILE ... FILE>>> block.";

const TRUNCATED_HINT: &str = "Previous turn was not executed because the raw file body was cut off (missing FILE>>>). Do not repeat the partial body. Resend the remaining text with home.write arguments {\"body\":\"raw\",\"mode\":\"append\"} and a complete <<<FILE newline file-bytes newline FILE>>> block.";

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub(crate) struct ToolCallDirective {
    pub(crate) server: String,
    pub(crate) tool: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ModelDirective {
    pub(crate) done: bool,
    #[allow(dead_code)]
    pub(crate) thought: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<ToolCallDirective>,
    #[serde(default)]
    pub(crate) result: Option<String>,
}

/// JSON object kept in history, plus the directive with raw file bytes already
/// copied into `arguments.content`.
#[derive(Debug, Clone)]
pub(crate) struct ParsedDirective {
    pub(crate) directive: ModelDirective,
    /// The JSON object only. A raw file body is not included.
    pub(crate) json_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DirectiveParseError {
    message: String,
    hint: String,
}

impl DirectiveParseError {
    fn new(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: hint.into(),
        }
    }

    #[must_use]
    pub(crate) fn hint(&self) -> &str {
        &self.hint
    }
}

impl std::fmt::Display for DirectiveParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<serde_json::Error> for DirectiveParseError {
    fn from(err: serde_json::Error) -> Self {
        Self::new(err.to_string(), GENERAL_HINT)
    }
}

pub(crate) fn parse_directive(raw: &str) -> Result<ParsedDirective, DirectiveParseError> {
    let prepared = strip_opening_fence(raw.trim());
    if !prepared.starts_with('{') {
        return Err(DirectiveParseError::new(
            "expected a JSON object",
            GENERAL_HINT,
        ));
    }
    let (json_text, rest) = split_json_object(prepared)?;
    let mut directive: ModelDirective = serde_json::from_str(json_text)?;
    attach_raw_body(&mut directive, rest)?;
    Ok(ParsedDirective {
        directive,
        json_text: json_text.to_string(),
    })
}

/// Rough count of top-level JSON objects in a model reply (detects multi-JSON turns).
pub(crate) fn approx_json_object_count(content: &str) -> usize {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let mut count = 0usize;
    if trimmed.starts_with('{') {
        count = count.saturating_add(1);
    }
    count = count.saturating_add(trimmed.matches("\n{").count());
    count
}

pub(crate) fn content_preview(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    let mut preview: String = trimmed.chars().take(max_chars).collect();
    if trimmed.chars().count() > max_chars {
        preview.push('…');
    }
    preview
}

fn strip_opening_fence(trimmed: &str) -> &str {
    let without = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    without.trim_start_matches(['\r', '\n'])
}

/// Returns `(object, remainder)` where `object` is the first JSON value and
/// `remainder` is everything after it.
fn split_json_object(input: &str) -> Result<(&str, &str), DirectiveParseError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (index, ch) in input.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = index + ch.len_utf8();
                    return Ok((&input[..end], &input[end..]));
                }
            }
            _ => {}
        }
    }
    Err(DirectiveParseError::new(
        "unterminated JSON object",
        GENERAL_HINT,
    ))
}

fn attach_raw_body(directive: &mut ModelDirective, rest: &str) -> Result<(), DirectiveParseError> {
    let mut raw_indexes = Vec::new();
    for (index, call) in directive.tool_calls.iter().enumerate() {
        if !is_home_write(call) {
            continue;
        }
        match call.arguments.get("body") {
            None | Some(Value::Null) => {}
            Some(Value::String(value)) if value == "raw" => {
                if has_content(&call.arguments) {
                    return Err(DirectiveParseError::new(
                        "home.write cannot set both content and body \"raw\"",
                        GENERAL_HINT,
                    ));
                }
                raw_indexes.push(index);
            }
            Some(_) => {
                return Err(DirectiveParseError::new(
                    "home.write body must be \"raw\" when present",
                    GENERAL_HINT,
                ));
            }
        }
    }

    if raw_indexes.is_empty() {
        let trailing = strip_closing_fence(rest);
        if !trailing.is_empty() {
            return Err(DirectiveParseError::new(
                "unexpected text after the JSON object",
                GENERAL_HINT,
            ));
        }
        return Ok(());
    }
    if raw_indexes.len() > 1 {
        return Err(DirectiveParseError::new(
            "only one raw home.write body is allowed per turn",
            GENERAL_HINT,
        ));
    }

    let body = extract_raw_body(rest)?;
    let index = raw_indexes[0];
    let Some(map) = directive.tool_calls[index].arguments.as_object_mut() else {
        return Err(DirectiveParseError::new(
            "home.write arguments must be an object",
            GENERAL_HINT,
        ));
    };
    map.remove("body");
    map.insert("content".to_string(), Value::String(body));
    Ok(())
}

fn is_home_write(call: &ToolCallDirective) -> bool {
    call.server == "home" && call.tool == "write"
}

fn has_content(arguments: &Value) -> bool {
    match arguments.get("content") {
        None | Some(Value::Null) => false,
        Some(_) => true,
    }
}

fn strip_closing_fence(rest: &str) -> &str {
    let trimmed = rest.trim();
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

fn extract_raw_body(rest: &str) -> Result<String, DirectiveParseError> {
    let tail = strip_closing_fence_prefix(rest.trim_start());
    let Some(after_marker) = tail.strip_prefix(FILE_BEGIN) else {
        return Err(DirectiveParseError::new(
            "raw home.write is missing the <<<FILE body",
            TRUNCATED_HINT,
        ));
    };
    let after_newline = after_marker
        .strip_prefix("\r\n")
        .or_else(|| after_marker.strip_prefix('\n'))
        .ok_or_else(|| {
            DirectiveParseError::new("<<<FILE must be followed by a newline", TRUNCATED_HINT)
        })?;
    let content = take_until_end_marker(after_newline).ok_or_else(|| {
        DirectiveParseError::new("raw file body is missing FILE>>>", TRUNCATED_HINT)
    })?;
    Ok(content.to_string())
}

/// Drops a closing markdown fence that some models place between the JSON
/// object and the raw body.
fn strip_closing_fence_prefix(tail: &str) -> &str {
    let Some(rest) = tail.strip_prefix("```") else {
        return tail;
    };
    rest.trim_start()
}

/// Content is everything before a line whose entire text is `FILE>>>`.
fn take_until_end_marker(body: &str) -> Option<&str> {
    if marker_line(body).is_some() {
        return Some("");
    }
    let mut search_from = 0usize;
    let bytes = body.as_bytes();
    while let Some(rel) = body[search_from..].find(FILE_END) {
        let abs = search_from + rel;
        let at_line_start = abs == 0 || bytes.get(abs.wrapping_sub(1)) == Some(&b'\n');
        if at_line_start {
            if marker_line(&body[abs..]).is_none() {
                search_from = abs + FILE_END.len();
                continue;
            }
            if abs == 0 {
                return Some("");
            }
            let mut end = abs - 1;
            if end > 0 && bytes.get(end - 1) == Some(&b'\r') {
                end -= 1;
            }
            return Some(&body[..end]);
        }
        search_from = abs + FILE_END.len();
    }
    None
}

/// `FILE>>>` at the start of `text`, optionally followed only by a newline.
fn marker_line(text: &str) -> Option<()> {
    let rest = text.strip_prefix(FILE_END)?;
    if rest.is_empty() || rest.starts_with('\n') || rest.starts_with("\r\n") {
        Some(())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_directive_accepts_plain_json() {
        let parsed =
            parse_directive(r#"{"done":true,"thought":"ok","tool_calls":[],"result":"finished"}"#)
                .expect("plain json should parse");

        assert!(parsed.directive.done);
        assert_eq!(parsed.directive.result.as_deref(), Some("finished"));
        assert!(parsed.directive.tool_calls.is_empty());
    }

    #[test]
    fn parse_directive_strips_markdown_fences() {
        let parsed = parse_directive(
            "```json\n{\"done\":false,\"thought\":\"step\",\"tool_calls\":[],\"result\":null}\n```",
        )
        .expect("fenced json should parse");

        assert!(!parsed.directive.done);
        assert_eq!(parsed.directive.thought.as_deref(), Some("step"));
    }

    #[test]
    fn parse_directive_rejects_invalid_json() {
        assert!(parse_directive("not json at all").is_err());
        assert!(parse_directive("```json\n{broken\n```").is_err());
    }

    #[test]
    fn parse_directive_rejects_trailing_second_object() {
        let multi = concat!(
            r#"{"done":false,"thought":"a","tool_calls":[],"result":null}"#,
            "\n\n",
            r#"{"done":true,"thought":"b","tool_calls":[],"result":"1"}"#,
        );
        let err = parse_directive(multi).expect_err("two objects");
        assert!(err.to_string().contains("unexpected text"));
    }

    #[test]
    fn parse_directive_extracts_raw_body_with_quotes() {
        let raw = concat!(
            r#"{"done":false,"thought":"write","tool_calls":[{"server":"home","tool":"write","arguments":{"path":"out/src/Module.bsl","body":"raw","mode":"create"}}],"result":null}"#,
            "\n<<<FILE\n",
            "Процедура Тест()\n\tСообщить(\"ok\");\nКонецПроцедуры\n",
            "FILE>>>\n",
        );
        let parsed = parse_directive(raw).expect("raw body");
        let args = &parsed.directive.tool_calls[0].arguments;
        assert_eq!(args["path"], "out/src/Module.bsl");
        assert_eq!(args["mode"], "create");
        assert!(args.get("body").is_none());
        assert_eq!(
            args["content"],
            "Процедура Тест()\n\tСообщить(\"ok\");\nКонецПроцедуры"
        );
        assert!(!parsed.json_text.contains("Сообщить"));
    }

    #[test]
    fn parse_directive_rejects_content_and_raw_body_together() {
        let raw = concat!(
            r#"{"done":false,"thought":"write","tool_calls":[{"server":"home","tool":"write","arguments":{"path":"out/a.bsl","content":"x","body":"raw"}}],"result":null}"#,
            "\n<<<FILE\nx\nFILE>>>\n",
        );
        let err = parse_directive(raw).expect_err("both");
        assert!(err.to_string().contains("both content and body"));
    }

    #[test]
    fn parse_directive_missing_end_marker_asks_for_append() {
        let raw = concat!(
            r#"{"done":false,"thought":"write","tool_calls":[{"server":"home","tool":"write","arguments":{"path":"out/a.bsl","body":"raw"}}],"result":null}"#,
            "\n<<<FILE\nПроцедура Тест()\n",
        );
        let err = parse_directive(raw).expect_err("truncated");
        assert!(err.to_string().contains("FILE>>>"));
        assert!(err.hint().contains("append"));
    }

    #[test]
    fn approx_json_object_count_detects_multi_json() {
        let multi = concat!(
            r#"{"done":false,"thought":"a","tool_calls":[],"result":null}"#,
            "\n\n",
            r#"{"done":true,"thought":"b","tool_calls":[],"result":"1"}"#,
        );
        assert_eq!(approx_json_object_count(multi), 2);
        assert_eq!(
            approx_json_object_count(r#"{"done":true,"tool_calls":[],"result":null}"#),
            1
        );
        assert_eq!(approx_json_object_count(""), 0);
    }

    #[test]
    fn content_preview_truncates() {
        let preview = content_preview("abcdefghij", 4);
        assert_eq!(preview, "abcd…");
    }
}
