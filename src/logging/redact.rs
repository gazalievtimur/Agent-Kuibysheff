//! Audit payload redaction and string truncation for structured EventSink logs.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::config::AuditRedactionConfig;

use super::sink::{EventSink, SharedEventSink, SinkDestination};
use super::LoggingError;

const DEFAULT_SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "authorization",
    "password",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "client_secret",
    "private_key",
    "cookie",
    "set-cookie",
    "bearer",
];

const REDACTED: &str = "[REDACTED]";
const TRUNCATED_SUFFIX: &str = "…[truncated]";

/// Runtime policy applied to structured audit payloads before they hit disk.
#[derive(Debug, Clone)]
pub struct AuditRedactionPolicy {
    pub enabled: bool,
    pub max_string_chars: usize,
    sensitive_keys: HashSet<String>,
}

impl Default for AuditRedactionPolicy {
    fn default() -> Self {
        Self::from_config(&AuditRedactionConfig::default())
    }
}

impl AuditRedactionPolicy {
    /// Builds a policy from logging configuration.
    #[must_use]
    pub fn from_config(config: &AuditRedactionConfig) -> Self {
        let mut sensitive_keys: HashSet<String> = DEFAULT_SENSITIVE_KEYS
            .iter()
            .map(|key| (*key).to_string())
            .collect();
        for key in &config.extra_sensitive_keys {
            let normalized = key.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                sensitive_keys.insert(normalized);
            }
        }
        Self {
            enabled: config.enabled,
            max_string_chars: config.max_string_chars.max(1),
            sensitive_keys,
        }
    }

    fn is_sensitive_key(&self, key: &str) -> bool {
        self.sensitive_keys.contains(&key.to_ascii_lowercase())
    }
}

/// Redacts sensitive keys and truncates long strings in an audit payload.
#[must_use]
pub fn redact_value(value: &Value, policy: &AuditRedactionPolicy) -> Value {
    if !policy.enabled {
        return value.clone();
    }
    redact_node(value, policy, false)
}

fn redact_node(value: &Value, policy: &AuditRedactionPolicy, parent_sensitive: bool) -> Value {
    if parent_sensitive {
        return Value::String(REDACTED.to_string());
    }
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (key, child) in map {
                let sensitive = policy.is_sensitive_key(key);
                out.insert(key.clone(), redact_node(child, policy, sensitive));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_node(item, policy, false))
                .collect(),
        ),
        Value::String(text) => Value::String(truncate_utf8_chars(text, policy.max_string_chars)),
        other => other.clone(),
    }
}

fn truncate_utf8_chars(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push_str(TRUNCATED_SUFFIX);
    truncated
}

/// Applies [`AuditRedactionPolicy`] to every payload before forwarding.
pub struct RedactingEventSink {
    inner: SharedEventSink,
    policy: AuditRedactionPolicy,
}

impl RedactingEventSink {
    #[must_use]
    pub fn wrap(inner: SharedEventSink, policy: AuditRedactionPolicy) -> SharedEventSink {
        Arc::new(Self { inner, policy })
    }
}

#[async_trait]
impl EventSink for RedactingEventSink {
    async fn write_event(&self, event_type: &str, payload: Value) -> Result<(), LoggingError> {
        let redacted = redact_value(&payload, &self.policy);
        self.inner.write_event(event_type, redacted).await
    }

    fn destination(&self) -> SinkDestination {
        self.inner.destination()
    }

    async fn shutdown(&self) -> Result<(), LoggingError> {
        self.inner.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::sink::MemoryEventSink;
    use serde_json::json;
    use std::sync::Arc;

    fn policy_enabled(max_string_chars: usize) -> AuditRedactionPolicy {
        AuditRedactionPolicy::from_config(&AuditRedactionConfig {
            enabled: true,
            max_string_chars,
            extra_sensitive_keys: Vec::new(),
        })
    }

    #[test]
    fn redacts_nested_sensitive_keys_case_insensitive() {
        let policy = policy_enabled(4096);
        let input = json!({
            "outer": {
                "API_KEY": "sk-secret",
                "Authorization": "Bearer abc",
                "ok": true
            }
        });
        let out = redact_value(&input, &policy);
        assert_eq!(out["outer"]["API_KEY"], REDACTED);
        assert_eq!(out["outer"]["Authorization"], REDACTED);
        assert_eq!(out["outer"]["ok"], true);
    }

    #[test]
    fn truncates_long_strings_with_suffix() {
        let policy = policy_enabled(4);
        let out = redact_value(&json!({ "content": "abcdefghij" }), &policy);
        assert_eq!(out["content"], format!("abcd{TRUNCATED_SUFFIX}"));
    }

    #[test]
    fn disabled_policy_passthrough() {
        let policy = AuditRedactionPolicy::from_config(&AuditRedactionConfig {
            enabled: false,
            max_string_chars: 2,
            extra_sensitive_keys: Vec::new(),
        });
        let input = json!({ "api_key": "secret", "content": "abcdefghij" });
        assert_eq!(redact_value(&input, &policy), input);
    }

    #[test]
    fn numbers_and_bools_untouched() {
        let policy = policy_enabled(8);
        let input = json!({ "tokens": 42, "ok": false, "ratio": 1.5 });
        assert_eq!(redact_value(&input, &policy), input);
    }

    #[test]
    fn extra_sensitive_keys_are_merged() {
        let policy = AuditRedactionPolicy::from_config(&AuditRedactionConfig {
            enabled: true,
            max_string_chars: 4096,
            extra_sensitive_keys: vec!["session_id".to_string()],
        });
        let out = redact_value(&json!({ "session_id": "abc", "tool": "x" }), &policy);
        assert_eq!(out["session_id"], REDACTED);
        assert_eq!(out["tool"], "x");
    }

    #[tokio::test]
    async fn redacting_sink_writes_sanitized_payload() {
        let memory = Arc::new(MemoryEventSink::new());
        let sink = RedactingEventSink::wrap(memory.clone(), policy_enabled(4));
        sink.write_event(
            "mcp_tool_call",
            json!({
                "api_key": "secret",
                "arguments": { "body": "abcdefghij" }
            }),
        )
        .await
        .expect("write");

        let events = memory.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "mcp_tool_call");
        assert_eq!(events[0].1["api_key"], REDACTED);
        assert_eq!(
            events[0].1["arguments"]["body"],
            format!("abcd{TRUNCATED_SUFFIX}")
        );
    }
}
