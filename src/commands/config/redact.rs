//! Secret redaction and merge for `config document` get/set.

use serde_json::{json, Map, Value};

use crate::config::{AppConfig, McpTransport};

const SET_MARKER: &str = "set";

/// Serialize config for the studio UI with secrets scrubbed.
#[must_use]
pub fn redact_config(cfg: &AppConfig) -> Value {
    let mut value = serde_json::to_value(cfg).unwrap_or(Value::Null);
    if let Some(provider) = value.get_mut("provider").and_then(Value::as_object_mut) {
        provider.insert("api_key".to_string(), Value::Null);
    }
    if let Some(servers) = value.get_mut("mcp").and_then(Value::as_array_mut) {
        for server in servers {
            redact_mcp_server(server);
        }
    }
    value
}

fn redact_mcp_server(server: &mut Value) {
    let Some(obj) = server.as_object_mut() else {
        return;
    };
    if let Some(env) = obj.get_mut("env").and_then(Value::as_object_mut) {
        redact_string_map(env);
    }
    if let Some(headers) = obj.get_mut("headers").and_then(Value::as_object_mut) {
        redact_string_map(headers);
    }
    if obj.get("auth").is_some_and(|a| !a.is_null()) {
        obj.insert("auth".to_string(), set_marker());
    }
}

fn redact_string_map(map: &mut Map<String, Value>) {
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let keep = map
            .get(&key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        if keep {
            map.insert(key, set_marker());
        } else {
            map.remove(&key);
        }
    }
}

fn set_marker() -> Value {
    json!({ SET_MARKER: true })
}

fn is_set_marker(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.get(SET_MARKER).and_then(Value::as_bool) == Some(true))
}

/// Merge a studio PUT body with the existing profile so null/`{set:true}` do not wipe secrets.
///
/// # Errors
///
/// Returns a human-readable message when the merged JSON is not a valid [`AppConfig`].
pub fn merge_config_put(
    existing: &AppConfig,
    mut incoming: Value,
    clear_api_key: bool,
) -> Result<AppConfig, String> {
    let Some(provider) = incoming.get_mut("provider").and_then(Value::as_object_mut) else {
        return Err("config.provider is required".to_string());
    };
    if clear_api_key {
        provider.insert("api_key".to_string(), Value::Null);
    } else {
        let incoming_key = provider.get("api_key").cloned().unwrap_or(Value::Null);
        match incoming_key {
            Value::Null => {
                if let Some(key) = existing.provider.api_key.as_ref() {
                    provider.insert("api_key".to_string(), Value::String(key.clone()));
                } else {
                    provider.insert("api_key".to_string(), Value::Null);
                }
            }
            Value::String(s) if s.trim().is_empty() => {
                if let Some(key) = existing.provider.api_key.as_ref() {
                    provider.insert("api_key".to_string(), Value::String(key.clone()));
                } else {
                    provider.insert("api_key".to_string(), Value::Null);
                }
            }
            other => {
                provider.insert("api_key".to_string(), other);
            }
        }
    }

    merge_mcp_array(existing, &mut incoming)?;

    serde_json::from_value(incoming).map_err(|err| format!("invalid config: {err}"))
}

fn merge_mcp_array(existing: &AppConfig, incoming: &mut Value) -> Result<(), String> {
    let Some(servers) = incoming.get_mut("mcp").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for server in servers.iter_mut() {
        let name = server
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "mcp server missing name".to_string())?
            .to_string();
        let Some(prev) = existing.mcp.iter().find(|s| s.name == name) else {
            // New server: reject set markers (nothing to restore).
            reject_unresolved_markers(server, &name)?;
            continue;
        };
        restore_mcp_secrets(prev, server)?;
    }
    Ok(())
}

fn reject_unresolved_markers(server: &Value, name: &str) -> Result<(), String> {
    if let Some(env) = server.get("env").and_then(Value::as_object) {
        for (k, v) in env {
            if is_set_marker(v) {
                return Err(format!(
                    "mcp `{name}` env `{k}` has a set-marker but the server is new"
                ));
            }
        }
    }
    if let Some(headers) = server.get("headers").and_then(Value::as_object) {
        for (k, v) in headers {
            if is_set_marker(v) {
                return Err(format!(
                    "mcp `{name}` header `{k}` has a set-marker but the server is new"
                ));
            }
        }
    }
    if server.get("auth").is_some_and(is_set_marker) {
        return Err(format!(
            "mcp `{name}` auth has a set-marker but the server is new"
        ));
    }
    Ok(())
}

fn restore_mcp_secrets(
    existing: &crate::config::McpServerConfig,
    server: &mut Value,
) -> Result<(), String> {
    let Some(obj) = server.as_object_mut() else {
        return Ok(());
    };
    match &existing.transport {
        McpTransport::Stdio(stdio) => {
            if let Some(env_val) = obj.get_mut("env") {
                restore_string_map(env_val, &stdio.env)?;
            }
        }
        McpTransport::Http(http) => {
            if let Some(headers_val) = obj.get_mut("headers") {
                restore_string_map(headers_val, &http.headers)?;
            }
            if obj.get("auth").is_some_and(is_set_marker) {
                match &http.auth {
                    Some(auth) => {
                        obj.insert(
                            "auth".to_string(),
                            serde_json::to_value(auth).map_err(|e| e.to_string())?,
                        );
                    }
                    None => {
                        obj.insert("auth".to_string(), Value::Null);
                    }
                }
            }
        }
    }
    Ok(())
}

fn restore_string_map(
    incoming: &mut Value,
    existing: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let Some(map) = incoming.as_object_mut() else {
        return Ok(());
    };
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let Some(value) = map.get(&key).cloned() else {
            continue;
        };
        if is_set_marker(&value) {
            let Some(prev) = existing.get(&key) else {
                return Err(format!(
                    "set-marker for `{key}` but no previous secret exists"
                ));
            };
            map.insert(key, Value::String(prev.clone()));
        }
    }
    Ok(())
}

/// Validate skill name/policy for DSL quoting (no bare `"`).
///
/// # Errors
///
/// Returns a message when a field contains a double quote.
pub fn validate_skill_fields(name: &str, policy: &str) -> Result<(), String> {
    if name.contains('"') {
        return Err("skill name must not contain `\"`".to_string());
    }
    if policy.contains('"') {
        return Err("skill policy must not contain `\"`".to_string());
    }
    if name.trim().is_empty() {
        return Err("skill name must not be empty".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::AccessPolicyConfig;
    use crate::config::{
        BillingConfig, LoggingConfig, McpHttpConfig, McpOAuthConfig, McpServerConfig,
        McpStdioConfig, McpTransport, ProviderConfig,
    };
    use crate::event_mcp::EventMcpConfig;
    use crate::limits::LimitsConfig;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn sample_config() -> AppConfig {
        let mut env = HashMap::new();
        env.insert("TOKEN".to_string(), "secret-token".to_string());
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer xyz".to_string());
        AppConfig {
            provider: ProviderConfig {
                base_url: "https://example.test/v1".to_string(),
                model: "gpt-test".to_string(),
                api_key_env: "TEST_KEY".to_string(),
                api_key: Some("inline-secret".to_string()),
                timeout_ms: 60_000,
                max_retries: 3,
                retry_base_delay_ms: 500,
                history: Default::default(),
            },
            mcp: vec![
                McpServerConfig {
                    name: "stdio".to_string(),
                    timeout_ms: 20_000,
                    transport: McpTransport::Stdio(McpStdioConfig {
                        command: "echo".to_string(),
                        args: vec![],
                        env,
                        cwd: None,
                    }),
                },
                McpServerConfig {
                    name: "http".to_string(),
                    timeout_ms: 20_000,
                    transport: McpTransport::Http(McpHttpConfig {
                        url: "https://mcp.example/mcp".to_string(),
                        headers,
                        auth: Some(McpOAuthConfig {
                            client_id: Some("cid".to_string()),
                            client_secret_env: Some("SECRET".to_string()),
                            client_id_metadata_url: None,
                            scopes: vec![],
                            redirect_port: 0,
                            token_store: Some(PathBuf::from("tokens.json")),
                        }),
                    }),
                },
            ],
            event_mcp: EventMcpConfig::default(),
            billing: BillingConfig::default(),
            limits: LimitsConfig {
                max_iterations: 5,
                max_tokens: 1000,
                max_duration_sec: 60,
                max_cost: None,
            },
            logging: LoggingConfig::default(),
            access: Some(AccessPolicyConfig::minimal_profile()),
            context: crate::config::ContextConfig::default(),
        }
    }

    #[test]
    fn redact_clears_api_key_and_marks_mcp_secrets() {
        let redacted = redact_config(&sample_config());
        assert!(redacted["provider"]["api_key"].is_null());
        assert_eq!(redacted["mcp"][0]["env"]["TOKEN"], set_marker());
        assert_eq!(redacted["mcp"][1]["headers"]["Authorization"], set_marker());
        assert_eq!(redacted["mcp"][1]["auth"], set_marker());
        assert!(!serde_json::to_string(&redacted)
            .unwrap()
            .contains("inline-secret"));
        assert!(!serde_json::to_string(&redacted)
            .unwrap()
            .contains("secret-token"));
    }

    #[test]
    fn null_api_key_does_not_clear_existing() {
        let existing = sample_config();
        let mut incoming = redact_config(&existing);
        incoming["provider"]["model"] = json!("other-model");
        let merged = merge_config_put(&existing, incoming, false).expect("merge");
        assert_eq!(merged.provider.api_key.as_deref(), Some("inline-secret"));
        assert_eq!(merged.provider.model, "other-model");
    }

    #[test]
    fn clear_api_key_flag_wipes_inline_key() {
        let existing = sample_config();
        let incoming = redact_config(&existing);
        let merged = merge_config_put(&existing, incoming, true).expect("merge");
        assert!(merged.provider.api_key.is_none());
    }

    #[test]
    fn set_marker_restores_mcp_env() {
        let existing = sample_config();
        let incoming = redact_config(&existing);
        let merged = merge_config_put(&existing, incoming, false).expect("merge");
        match &merged.mcp[0].transport {
            McpTransport::Stdio(stdio) => {
                assert_eq!(
                    stdio.env.get("TOKEN").map(String::as_str),
                    Some("secret-token")
                );
            }
            McpTransport::Http(_) => panic!("expected stdio"),
        }
        match &merged.mcp[1].transport {
            McpTransport::Http(http) => {
                assert_eq!(
                    http.headers.get("Authorization").map(String::as_str),
                    Some("Bearer xyz")
                );
                assert!(http.auth.is_some());
            }
            McpTransport::Stdio(_) => panic!("expected http"),
        }
    }

    #[test]
    fn skill_fields_reject_quotes() {
        assert!(validate_skill_fields("ok", "policy").is_ok());
        assert!(validate_skill_fields("bad\"name", "policy").is_err());
        assert!(validate_skill_fields("ok", "bad \"policy").is_err());
    }
}
