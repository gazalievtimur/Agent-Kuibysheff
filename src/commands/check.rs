//! Probe configured agent resources (provider, MCP, access paths, settings).

use std::fmt;
use std::io;
use std::path::Path;

use thiserror::Error;

use crate::access::AccessMode;
use crate::billing::PricingCatalog;
use crate::cli::CheckArgs;
use crate::config::{load_config, AppConfig, ConfigError, McpTransport};
use crate::logging::resolve_base_dir;
use crate::mcp::stdio_client::McpRegistry;
use crate::project_paths::{resolve_agent_identity, AgentPathError};
use crate::provider::openai_compat::OpenAiCompatClient;
use crate::sandbox::SandboxRunner;
use crate::settings::{load_settings, SettingsError};
use crate::skills::dsl::SkillsCatalog;
use crate::tool_api::ToolExecutor;
use tokio_util::sync::CancellationToken;

/// Fatal errors that prevent producing a check report.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CheckError {
    #[error("failed to load config: {0}")]
    Config(#[from] ConfigError),
    #[error(transparent)]
    AgentPath(#[from] AgentPathError),
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
    let paths = resolve_agent_identity(&args.identity.project_root, &args.identity.agent, None)?;
    let config_path = paths.config.clone();
    let settings_dir = paths.settings_dir.clone();
    let (cfg, access) = tokio::task::spawn_blocking(move || load_config(&config_path))
        .await
        .map_err(|err| {
            CheckError::Runtime(io::Error::other(format!("config load task: {err}")))
        })??;

    let mut items = Vec::new();
    items.push(CheckItem {
        name: "config".to_string(),
        status: CheckStatus::Ok,
        detail: format!(
            "loaded agent `{}` ({} MCP server(s))",
            args.identity.agent,
            cfg.mcp.len()
        ),
    });

    check_provider(&cfg, args.skip_provider, &mut items).await;
    check_mcp(
        &cfg,
        &paths.config,
        &args.identity.project_root,
        &args.identity.agent,
        args.skip_mcp,
        &mut items,
    )
    .await;
    check_billing_catalog(&cfg, &paths.config, &mut items);
    check_access(&access, &mut items);
    check_sandbox(&access, args.skip_sandbox, &mut items);
    check_logging(&cfg, &mut items);
    check_settings(&settings_dir, &mut items);

    Ok(CheckReport {
        config_path: paths.config.display().to_string(),
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

async fn check_mcp(
    cfg: &AppConfig,
    config_path: &Path,
    project_root: &Path,
    agent_id: &str,
    skip: bool,
    items: &mut Vec<CheckItem>,
) {
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

    let config_dir = config_path.parent();
    let _ = config_dir;
    for server in &cfg.mcp {
        let transport = match &server.transport {
            McpTransport::Stdio(stdio) => format!("stdio `{}`", stdio.command),
            McpTransport::Http(http) => format!("http `{}`", http.url),
        };
        let name = format!("mcp.{}", server.name);
        match McpRegistry::connect_all_isolated(
            std::slice::from_ref(server),
            None,
            CancellationToken::new(),
            crate::mcp::McpIsolationContext {
                project_root: Some(project_root.to_path_buf()),
                agent_id: agent_id.to_string(),
            },
        )
        .await
        {
            Ok(registry) => {
                let tools = registry.available_tools();
                let missing_billing_target = cfg
                    .billing
                    .mcp
                    .as_ref()
                    .filter(|binding| binding.target.starts_with(&format!("{}.", server.name)))
                    .is_some_and(|binding| !tools.iter().any(|tool| tool == &binding.target));
                items.push(CheckItem {
                    name,
                    status: if missing_billing_target {
                        CheckStatus::Fail
                    } else {
                        CheckStatus::Ok
                    },
                    detail: if missing_billing_target {
                        format!(
                            "{transport}; configured billing target was not discovered ({} tool(s))",
                            tools.len()
                        )
                    } else {
                        format!("{transport}; {} tool(s)", tools.len())
                    },
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

fn check_billing_catalog(cfg: &AppConfig, config_path: &Path, items: &mut Vec<CheckItem>) {
    let Some(path) = &cfg.billing.catalog_path else {
        items.push(CheckItem {
            name: "billing.catalog".to_string(),
            status: CheckStatus::Skip,
            detail: "no local pricing catalog configured".to_string(),
        });
        return;
    };
    let resolved = if path.is_absolute() {
        path.clone()
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    };
    match PricingCatalog::load(&resolved) {
        Ok(catalog) => items.push(CheckItem {
            name: "billing.catalog".to_string(),
            status: CheckStatus::Ok,
            detail: format!(
                "loaded `{}` version `{}` ({} rule(s))",
                resolved.display(),
                catalog.version,
                catalog.rules.len()
            ),
        }),
        Err(error) => items.push(CheckItem {
            name: "billing.catalog".to_string(),
            status: CheckStatus::Fail,
            detail: error.to_string(),
        }),
    }
}

fn check_access(access: &crate::access::ResolvedAccessPolicy, items: &mut Vec<CheckItem>) {
    match access.mode() {
        AccessMode::Legacy => {
            items.push(CheckItem {
                name: "access".to_string(),
                status: CheckStatus::Ok,
                detail: "legacy mode (`access.mode: legacy`)".to_string(),
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
                SettingsError::WriteFile { path, source } => {
                    format!("failed to write `{path}`: {source}")
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
    use crate::cli::AgentIdentityArgs;
    use crate::cli::InitArgs;
    use crate::commands::init;
    use crate::project_paths::AGENT_CONFIG_FILE;
    use std::fs;
    use tempfile::tempdir;

    fn scaffold_agent(project: &Path, agent: &str) {
        init::run(&InitArgs {
            agent_id: agent.to_string(),
            project_root: project.to_path_buf(),
            force: true,
            interactive: false,
        })
        .expect("init");
    }

    fn check_args(project: &Path, agent: &str) -> CheckArgs {
        CheckArgs {
            identity: AgentIdentityArgs {
                project_root: project.to_path_buf(),
                agent: agent.to_string(),
            },
            skip_provider: true,
            skip_mcp: true,
            skip_sandbox: true,
        }
    }

    #[test]
    fn check_passes_for_init_profile() {
        let dir = tempdir().expect("tempdir");
        scaffold_agent(dir.path(), "demo");
        // Starter config may lack a resolvable API key — skip provider probe.
        let report = run(&check_args(dir.path(), "demo")).expect("check");
        assert!(
            report
                .items
                .iter()
                .any(|i| i.name == "config" && i.status == CheckStatus::Ok),
            "{report:?}"
        );
        assert!(
            report
                .items
                .iter()
                .any(|i| i.name == "settings" && i.status == CheckStatus::Ok),
            "{report:?}"
        );
    }

    #[test]
    fn check_fails_when_api_key_missing() {
        let dir = tempdir().expect("tempdir");
        scaffold_agent(dir.path(), "demo");
        let paths = resolve_agent_identity(dir.path(), "demo", None).unwrap();
        fs::write(
            &paths.config,
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

access:
  mode: legacy
"#,
        )
        .expect("write");
        std::env::remove_var("CHECK_MISSING_KEY_UNLIKELY_SET_xyz");
        let mut args = check_args(dir.path(), "demo");
        args.skip_provider = true;
        let report = run(&args).expect("check");
        assert!(!report.all_passed());
        assert!(report
            .items
            .iter()
            .any(|i| { i.name == "provider.api_key" && i.status == CheckStatus::Fail }));
        let _ = AGENT_CONFIG_FILE;
    }
}
