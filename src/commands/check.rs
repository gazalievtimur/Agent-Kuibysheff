//! Probe configured agent resources (provider, MCP, access paths, settings).

use std::fmt;
use std::io;
use std::path::Path;

use thiserror::Error;

use crate::access::AccessMode;
use crate::cli::CheckArgs;
use crate::config::{load_config, AppConfig, ConfigError, McpTransport};
use crate::logging::resolve_base_dir;
use crate::mcp::stdio_client::McpRegistry;
use crate::provider::openai_compat::OpenAiCompatClient;
use crate::sandbox::SandboxRunner;
use crate::settings::{load_settings, SettingsError};
use crate::skills::dsl::SkillsCatalog;
use crate::tool_api::ToolExecutor;

/// Fatal errors that prevent producing a check report.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CheckError {
    #[error("failed to load config: {0}")]
    Config(#[from] ConfigError),
    #[error("failed to start async runtime: {0}")]
    Runtime(#[source] io::Error),
}

/// Outcome of one resource probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Fail,
    Skip,
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => write!(f, "ok"),
            Self::Fail => write!(f, "fail"),
            Self::Skip => write!(f, "skip"),
        }
    }
}

/// One named probe result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

/// Aggregate report for `check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub config_path: String,
    pub items: Vec<CheckItem>,
}

impl CheckReport {
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.items
            .iter()
            .all(|item| item.status != CheckStatus::Fail)
    }

    #[must_use]
    pub fn passed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == CheckStatus::Ok)
            .count()
    }

    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == CheckStatus::Fail)
            .count()
    }

    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == CheckStatus::Skip)
            .count()
    }
}

/// Load config and probe configured resources.
///
/// # Errors
///
/// Returns [`CheckError`] when the config cannot be loaded or the Tokio runtime
/// cannot be started. Individual probe failures are recorded in the report.
pub fn run(args: &CheckArgs) -> Result<CheckReport, CheckError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(CheckError::Runtime)?;
    runtime.block_on(run_async(args))
}

async fn run_async(args: &CheckArgs) -> Result<CheckReport, CheckError> {
    let config_path = args.config.clone();
    let (cfg, access) = tokio::task::spawn_blocking(move || load_config(&config_path))
        .await
        .map_err(|err| ConfigError::Validation(format!("config load task: {err}")))??;

    let mut items = Vec::new();
    items.push(CheckItem {
        name: "config".to_string(),
        status: CheckStatus::Ok,
        detail: format!(
            "loaded `{}` ({} MCP server(s))",
            args.config.display(),
            cfg.mcp.len()
        ),
    });

    check_provider(&cfg, args.skip_provider, &mut items).await;
    check_mcp(&cfg, args.skip_mcp, &mut items).await;
    check_access(&access, &mut items);
    check_sandbox(&access, args.skip_sandbox, &mut items);
    check_logging(&cfg, &mut items);

    if let Some(settings_dir) = &args.settings_dir {
        check_settings(settings_dir, &mut items);
    }

    Ok(CheckReport {
        config_path: args.config.display().to_string(),
        items,
    })
}

async fn check_provider(cfg: &AppConfig, skip: bool, items: &mut Vec<CheckItem>) {
    match cfg.provider.resolve_api_key() {
        Ok(_) => {
            let source = if cfg.provider.has_inline_api_key() {
                "inline `provider.api_key`".to_string()
            } else {
                format!("env `{}`", cfg.provider.api_key_env)
            };
            items.push(CheckItem {
                name: "provider.api_key".to_string(),
                status: CheckStatus::Ok,
                detail: format!("resolved from {source}"),
            });
        }
        Err(_) => {
            items.push(CheckItem {
                name: "provider.api_key".to_string(),
                status: CheckStatus::Fail,
                detail: format!("missing inline key and env `{}`", cfg.provider.api_key_env),
            });
            if !skip {
                items.push(CheckItem {
                    name: "provider.http".to_string(),
                    status: CheckStatus::Skip,
                    detail: "skipped because API key is missing".to_string(),
                });
            }
            return;
        }
    }

    if skip {
        items.push(CheckItem {
            name: "provider.http".to_string(),
            status: CheckStatus::Skip,
            detail: "skipped (`--skip-provider`)".to_string(),
        });
        return;
    }

    match OpenAiCompatClient::new(cfg.provider.clone()) {
        Ok(client) => match client.probe().await {
            Ok(detail) => items.push(CheckItem {
                name: "provider.http".to_string(),
                status: CheckStatus::Ok,
                detail,
            }),
            Err(err) => items.push(CheckItem {
                name: "provider.http".to_string(),
                status: CheckStatus::Fail,
                detail: err.to_string(),
            }),
        },
        Err(err) => items.push(CheckItem {
            name: "provider.http".to_string(),
            status: CheckStatus::Fail,
            detail: err.to_string(),
        }),
    }
}

async fn check_mcp(cfg: &AppConfig, skip: bool, items: &mut Vec<CheckItem>) {
    if skip {
        items.push(CheckItem {
            name: "mcp".to_string(),
            status: CheckStatus::Skip,
            detail: "skipped (`--skip-mcp`)".to_string(),
        });
        return;
    }

    if cfg.mcp.is_empty() {
        items.push(CheckItem {
            name: "mcp".to_string(),
            status: CheckStatus::Ok,
            detail: "no MCP servers configured".to_string(),
        });
        return;
    }

    for server in &cfg.mcp {
        let transport = match &server.transport {
            McpTransport::Stdio(stdio) => format!("stdio `{}`", stdio.command),
            McpTransport::Http(http) => format!("http `{}`", http.url),
        };
        let name = format!("mcp.{}", server.name);
        match McpRegistry::connect_all(std::slice::from_ref(server), None).await {
            Ok(registry) => {
                let tools = registry.available_tools();
                items.push(CheckItem {
                    name,
                    status: CheckStatus::Ok,
                    detail: format!("{transport}; {} tool(s)", tools.len()),
                });
                registry.shutdown().await;
            }
            Err(err) => items.push(CheckItem {
                name,
                status: CheckStatus::Fail,
                detail: format!("{transport}: {err}"),
            }),
        }
    }
}

fn check_access(access: &crate::access::ResolvedAccessPolicy, items: &mut Vec<CheckItem>) {
    match access.mode() {
        AccessMode::Legacy => {
            items.push(CheckItem {
                name: "access".to_string(),
                status: CheckStatus::Ok,
                detail: "legacy mode (no `access` section)".to_string(),
            });
        }
        AccessMode::Strict => {
            let mut parts = Vec::new();
            if let Some(workspace) = access.workspace() {
                parts.push(format!(
                    "workspace=`{}`",
                    workspace.root.as_path().display()
                ));
            }
            if !access.input_roots().is_empty() {
                parts.push(format!("{} input root(s)", access.input_roots().len()));
            }
            parts.push(format!("{} program(s)", access.programs().len()));
            items.push(CheckItem {
                name: "access".to_string(),
                status: CheckStatus::Ok,
                detail: format!("strict; {}", parts.join("; ")),
            });
            for program in access.programs().values() {
                let missing: Vec<_> = program
                    .inherit_env
                    .iter()
                    .filter(|key| std::env::var_os(key).is_none())
                    .cloned()
                    .collect();
                if missing.is_empty() {
                    items.push(CheckItem {
                        name: format!("access.run.{}", program.alias),
                        status: CheckStatus::Ok,
                        detail: format!("executable `{}`", program.executable.as_path().display()),
                    });
                } else {
                    items.push(CheckItem {
                        name: format!("access.run.{}", program.alias),
                        status: CheckStatus::Fail,
                        detail: format!(
                            "executable ok, missing inherit_env: {}",
                            missing.join(", ")
                        ),
                    });
                }
            }
        }
    }
}

fn check_sandbox(
    access: &crate::access::ResolvedAccessPolicy,
    skip: bool,
    items: &mut Vec<CheckItem>,
) {
    if access.programs().is_empty() {
        return;
    }
    if skip {
        items.push(CheckItem {
            name: "sandbox".to_string(),
            status: CheckStatus::Skip,
            detail: "skipped (`--skip-sandbox`)".to_string(),
        });
        return;
    }

    let runner = SandboxRunner::platform_default();
    match runner.probe() {
        Ok(()) => items.push(CheckItem {
            name: "sandbox".to_string(),
            status: CheckStatus::Ok,
            detail: "OS sandbox available".to_string(),
        }),
        Err(err) => items.push(CheckItem {
            name: "sandbox".to_string(),
            status: CheckStatus::Fail,
            detail: err.to_string(),
        }),
    }
}

fn check_logging(cfg: &AppConfig, items: &mut Vec<CheckItem>) {
    match resolve_base_dir(&cfg.logging) {
        Ok(dir) => items.push(CheckItem {
            name: "logging".to_string(),
            status: CheckStatus::Ok,
            detail: format!("base dir `{}`", dir.display()),
        }),
        Err(err) => items.push(CheckItem {
            name: "logging".to_string(),
            status: CheckStatus::Fail,
            detail: err.to_string(),
        }),
    }
}

fn check_settings(settings_dir: &Path, items: &mut Vec<CheckItem>) {
    match load_settings(settings_dir) {
        Ok(settings) => match SkillsCatalog::parse(&settings.skills_source) {
            Ok(catalog) => {
                let rules = if settings.rules.trim().is_empty() {
                    "rules.md absent/empty"
                } else {
                    "rules.md present"
                };
                items.push(CheckItem {
                    name: "settings".to_string(),
                    status: CheckStatus::Ok,
                    detail: format!(
                        "`{}`; skills ok ({} allowed tool(s)); {rules}",
                        settings_dir.display(),
                        catalog.allowed_qualified_tools().len()
                    ),
                });
            }
            Err(err) => items.push(CheckItem {
                name: "settings".to_string(),
                status: CheckStatus::Fail,
                detail: format!("skills.dsl parse error: {err}"),
            }),
        },
        Err(err) => items.push(CheckItem {
            name: "settings".to_string(),
            status: CheckStatus::Fail,
            detail: match err {
                SettingsError::ReadFile { path, source } => {
                    format!("failed to read `{path}`: {source}")
                }
                SettingsError::EmptyFile(path) => format!("`{path}` is empty"),
            },
        }),
    }
}

/// Print a human-readable check report to stdout.
pub fn print_report(report: &CheckReport) {
    println!("Checking `{}`...\n", report.config_path);
    let width = report
        .items
        .iter()
        .map(|item| item.name.len())
        .max()
        .unwrap_or(8);
    for item in &report.items {
        println!(
            "  [{:<4}] {:width$}  {}",
            item.status,
            item.name,
            item.detail,
            width = width
        );
    }
    println!();
    if report.all_passed() {
        println!(
            "Result: all checks passed ({} ok, {} skipped)",
            report.passed_count(),
            report.skipped_count()
        );
    } else {
        println!(
            "Result: {} failed, {} passed, {} skipped",
            report.failed_count(),
            report.passed_count(),
            report.skipped_count()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_minimal_config(dir: &Path, api_key_env: &str) -> std::path::PathBuf {
        let path = dir.join("agent-config.yaml");
        fs::write(
            &path,
            format!(
                r#"provider:
  base_url: "https://example.test/v1"
  model: "test-model"
  api_key_env: "{api_key_env}"
  api_key: "test-key"
  timeout_ms: 1000
  max_retries: 0
  retry_base_delay_ms: 1

mcp: []

limits:
  max_iterations: 1
  max_tokens: 100
  max_duration_sec: 10

logging:
  enable_ai_log: false
  enable_mcp_log: false
  enable_chat_history: false
  sink:
    type: file
"#
            ),
        )
        .expect("write config");
        path
    }

    #[test]
    fn check_passes_for_minimal_config_with_skip_provider() {
        let dir = tempdir().expect("tempdir");
        let config = write_minimal_config(dir.path(), "CHECK_TEST_KEY");
        let args = CheckArgs {
            config,
            settings_dir: None,
            skip_provider: true,
            skip_mcp: false,
            skip_sandbox: false,
        };
        let report = run(&args).expect("check");
        assert!(report.all_passed(), "{report:?}");
        assert!(report.items.iter().any(|i| i.name == "config"));
        assert!(report.items.iter().any(|i| i.name == "provider.api_key"));
        assert!(report
            .items
            .iter()
            .any(|i| i.name == "provider.http" && i.status == CheckStatus::Skip));
        assert!(report
            .items
            .iter()
            .any(|i| i.name == "mcp" && i.status == CheckStatus::Ok));
    }

    #[test]
    fn check_reports_settings_when_provided() {
        let dir = tempdir().expect("tempdir");
        let config = write_minimal_config(dir.path(), "CHECK_TEST_KEY");
        let settings = dir.path().join("settings");
        fs::create_dir_all(&settings).expect("mkdir");
        fs::write(settings.join("master_prompt.md"), "master").expect("master");
        fs::write(
            settings.join("skills.dsl"),
            r#"skill "x" { policy: "safe" allowed_tools: ["home.read"] }"#,
        )
        .expect("skills");
        let args = CheckArgs {
            config,
            settings_dir: Some(settings),
            skip_provider: true,
            skip_mcp: true,
            skip_sandbox: true,
        };
        let report = run(&args).expect("check");
        assert!(report.all_passed(), "{report:?}");
        let settings_item = report
            .items
            .iter()
            .find(|i| i.name == "settings")
            .expect("settings item");
        assert_eq!(settings_item.status, CheckStatus::Ok);
    }

    #[test]
    fn check_fails_when_api_key_missing() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("agent-config.yaml");
        fs::write(
            &path,
            r#"provider:
  base_url: "https://example.test/v1"
  model: "test-model"
  api_key_env: "CHECK_MISSING_KEY_UNLIKELY_SET_xyz"
  timeout_ms: 1000
  max_retries: 0
  retry_base_delay_ms: 1

mcp: []

limits:
  max_iterations: 1
  max_tokens: 100
  max_duration_sec: 10

logging:
  enable_ai_log: false
  enable_mcp_log: false
  enable_chat_history: false
  sink:
    type: file
"#,
        )
        .expect("write");
        // Ensure the env var is unset for this process.
        std::env::remove_var("CHECK_MISSING_KEY_UNLIKELY_SET_xyz");
        let args = CheckArgs {
            config: path,
            settings_dir: None,
            skip_provider: true,
            skip_mcp: true,
            skip_sandbox: true,
        };
        let report = run(&args).expect("check");
        assert!(!report.all_passed());
        assert!(report
            .items
            .iter()
            .any(|i| { i.name == "provider.api_key" && i.status == CheckStatus::Fail }));
    }
}
