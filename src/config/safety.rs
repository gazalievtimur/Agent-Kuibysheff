//! Extra safety checks for config import and save paths.

use std::path::{Component, Path};

use crate::project_paths::{path_contains_protected_segment, KUIBYSHEFF_DIR, PROTECTED_DIR};

use super::{AppConfig, ConfigError, McpTransport};

/// Validates config constraints that apply on import/save (beyond schema [`super::validate`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfigSafetyValidator;

impl ConfigSafetyValidator {
    /// Runs import/save safety checks against an already-parsed config.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Validation`] when an inline API key is present or an MCP
    /// `cwd` lexically targets the protected store (including via parent components).
    pub fn check(cfg: &AppConfig) -> Result<(), ConfigError> {
        if cfg.provider.has_inline_api_key() {
            return Err(ConfigError::Validation(
                "inline `provider.api_key` is not allowed on import/save; use `provider.api_key_env`"
                    .to_string(),
            ));
        }

        for server in &cfg.mcp {
            let McpTransport::Stdio(stdio) = &server.transport else {
                continue;
            };
            let Some(cwd) = &stdio.cwd else {
                continue;
            };
            check_mcp_cwd(&server.name, cwd)?;
        }

        Ok(())
    }
}

fn check_mcp_cwd(server_name: &str, cwd: &Path) -> Result<(), ConfigError> {
    if cwd.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ConfigError::Validation(format!(
            "`mcp[{server_name}].cwd` must not contain parent path components (`..`)"
        )));
    }

    if path_contains_protected_segment(cwd) {
        return Err(ConfigError::Validation(format!(
            "`mcp[{server_name}].cwd` must not point into the agent protected store (`{KUIBYSHEFF_DIR}/{PROTECTED_DIR}`)"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::config::{
        AccessPolicyConfig, BillingConfig, LoggingConfig, McpServerConfig, McpStdioConfig,
        ProviderConfig, ProviderHistoryConfig,
    };
    use crate::limits::LimitsConfig;

    fn sample() -> AppConfig {
        AppConfig {
            provider: ProviderConfig {
                base_url: "https://example.com/v1".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                api_key: None,
                timeout_ms: 1000,
                max_retries: 2,
                retry_base_delay_ms: 100,
                history: ProviderHistoryConfig::default(),
            },
            mcp: vec![],
            event_mcp: crate::event_mcp::EventMcpConfig::default(),
            billing: BillingConfig::default(),
            limits: LimitsConfig {
                max_iterations: 5,
                max_tokens: 500,
                max_duration_sec: 30,
                max_cost: None,
            },
            logging: LoggingConfig::default(),
            access: Some(AccessPolicyConfig::default()),
        }
    }

    #[test]
    fn rejects_inline_api_key() {
        let mut cfg = sample();
        cfg.provider.api_key = Some("secret".to_string());
        let err = ConfigSafetyValidator::check(&cfg).expect_err("inline key");
        assert!(err.to_string().contains("api_key"), "{err}");
    }

    #[test]
    fn rejects_mcp_cwd_with_parent_into_protected() {
        let mut cfg = sample();
        cfg.mcp.push(McpServerConfig {
            name: "local".to_string(),
            timeout_ms: 1000,
            transport: McpTransport::Stdio(McpStdioConfig {
                command: "mcp".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: Some(PathBuf::from("../.kuibysheff/protected")),
            }),
        });
        let err = ConfigSafetyValidator::check(&cfg).expect_err("parent cwd");
        assert!(
            err.to_string().contains("parent") || err.to_string().contains("protected"),
            "{err}"
        );
    }

    #[test]
    fn rejects_mcp_cwd_pointing_at_protected() {
        let mut cfg = sample();
        cfg.mcp.push(McpServerConfig {
            name: "local".to_string(),
            timeout_ms: 1000,
            transport: McpTransport::Stdio(McpStdioConfig {
                command: "mcp".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: Some(PathBuf::from(".kuibysheff/protected/agents/x")),
            }),
        });
        let err = ConfigSafetyValidator::check(&cfg).expect_err("protected cwd");
        assert!(err.to_string().contains("protected"), "{err}");
    }
}
