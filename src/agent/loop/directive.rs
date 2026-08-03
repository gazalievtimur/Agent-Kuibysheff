use serde::Deserialize;
use serde_json::Value;

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

pub(crate) fn parse_directive(raw: &str) -> Result<ModelDirective, serde_json::Error> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed);
    }

    // Some models still wrap JSON in Markdown fences.
    let stripped = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    serde_json::from_str(stripped)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_directive_accepts_plain_json() {
        let directive =
            parse_directive(r#"{"done":true,"thought":"ok","tool_calls":[],"result":"finished"}"#)
                .expect("plain json should parse");

        assert!(directive.done);
        assert_eq!(directive.result.as_deref(), Some("finished"));
        assert!(directive.tool_calls.is_empty());
    }

    #[test]
    fn parse_directive_strips_markdown_fences() {
        let directive = parse_directive(
            "```json\n{\"done\":false,\"thought\":\"step\",\"tool_calls\":[],\"result\":null}\n```",
        )
        .expect("fenced json should parse");

        assert!(!directive.done);
        assert_eq!(directive.thought.as_deref(), Some("step"));
    }

    #[test]
    fn parse_directive_rejects_invalid_json() {
        assert!(parse_directive("not json at all").is_err());
        assert!(parse_directive("```json\n{broken\n```").is_err());
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
