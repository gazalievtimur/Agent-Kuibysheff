//! `config mcp` list/get/add/set/remove/tools.

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::cli::{ConfigArgs, McpCmd};
use crate::config::{McpHttpConfig, McpServerConfig, McpStdioConfig, McpTransport};
use crate::mcp::stdio_client::{McpIsolationContext, McpRegistry};
use crate::tool_api::ToolExecutor;

use super::common::{load_profile, parse_kv_pairs, save_profile_config};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct McpList {
    servers: Vec<McpServerView>,
}

#[derive(Debug, Serialize)]
struct McpServerView {
    name: String,
    transport: String,
    timeout_ms: u64,
    command: Option<String>,
    args: Option<Vec<String>>,
    url: Option<String>,
    cwd: Option<String>,
}

#[derive(Debug, Serialize)]
struct McpToolsView {
    tools: Vec<String>,
}

pub fn run(args: &ConfigArgs, cmd: &McpCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        McpCmd::List => {
            let profile = load_profile(&args.identity)?;
            let servers = profile
                .config
                .mcp
                .iter()
                .map(server_view)
                .collect::<Vec<_>>();
            emit_ok(args.format, "mcp", "list", &McpList { servers })
        }
        McpCmd::Get { name } => {
            let profile = load_profile(&args.identity)?;
            let server = profile
                .config
                .mcp
                .iter()
                .find(|s| s.name == *name)
                .ok_or_else(|| ConfigCmdError::message(format!("MCP server `{name}` not found")))?;
            emit_ok(args.format, "mcp", "get", &server_view(server))
        }
        McpCmd::Add {
            name,
            command,
            args: cmd_args,
            url,
            env,
            headers,
            cwd,
            timeout_ms,
        } => {
            let mut profile = load_profile(&args.identity)?;
            if profile.config.mcp.iter().any(|s| s.name == *name) {
                return Err(ConfigCmdError::message(format!(
                    "MCP server `{name}` already exists"
                )));
            }
            let server = build_server(
                name.clone(),
                command.as_deref(),
                cmd_args,
                url.as_deref(),
                env,
                headers,
                cwd.clone(),
                *timeout_ms,
            )?;
            profile.config.mcp.push(server);
            save_profile_config(&profile.paths, &profile.config)?;
            let view = server_view(profile.config.mcp.last().expect("just pushed"));
            emit_ok(args.format, "mcp", "add", &view)
        }
        McpCmd::Set {
            name,
            command,
            args: cmd_args,
            url,
            timeout_ms,
        } => {
            let mut profile = load_profile(&args.identity)?;
            let idx = profile
                .config
                .mcp
                .iter()
                .position(|s| s.name == *name)
                .ok_or_else(|| ConfigCmdError::message(format!("MCP server `{name}` not found")))?;
            let existing = &profile.config.mcp[idx];
            let mut view_command = None;
            let mut view_args = Vec::new();
            let mut view_url = None;
            let mut view_env = Default::default();
            let mut view_headers = Default::default();
            let mut view_cwd = None;
            match &existing.transport {
                McpTransport::Stdio(stdio) => {
                    view_command = Some(stdio.command.clone());
                    view_args = stdio.args.clone();
                    view_env = stdio.env.clone();
                    view_cwd = stdio.cwd.clone();
                }
                McpTransport::Http(http) => {
                    view_url = Some(http.url.clone());
                    view_headers = http.headers.clone();
                }
            }
            if let Some(c) = command {
                view_command = Some(c.clone());
                view_url = None;
            }
            if !cmd_args.is_empty() {
                view_args = cmd_args.clone();
            }
            if let Some(u) = url {
                view_url = Some(u.clone());
                view_command = None;
                view_args.clear();
                view_env.clear();
                view_cwd = None;
            }
            let timeout = timeout_ms.unwrap_or(existing.timeout_ms);
            let updated = build_server(
                name.clone(),
                view_command.as_deref(),
                &view_args,
                view_url.as_deref(),
                &env_pairs_from_map(&view_env),
                &env_pairs_from_map(&view_headers),
                view_cwd,
                Some(timeout),
            )?;
            profile.config.mcp[idx] = updated;
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(
                args.format,
                "mcp",
                "set",
                &server_view(&profile.config.mcp[idx]),
            )
        }
        McpCmd::Remove { name } => {
            let mut profile = load_profile(&args.identity)?;
            let before = profile.config.mcp.len();
            profile.config.mcp.retain(|s| s.name != *name);
            if profile.config.mcp.len() == before {
                return Err(ConfigCmdError::message(format!(
                    "MCP server `{name}` not found"
                )));
            }
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(
                args.format,
                "mcp",
                "remove",
                &serde_json::json!({ "name": name }),
            )
        }
        McpCmd::Tools { server } => {
            let profile = load_profile(&args.identity)?;
            let configs: Vec<_> = if let Some(name) = server {
                let one = profile
                    .config
                    .mcp
                    .iter()
                    .find(|s| s.name == *name)
                    .ok_or_else(|| {
                        ConfigCmdError::message(format!("MCP server `{name}` not found"))
                    })?;
                vec![one.clone()]
            } else {
                profile.config.mcp.clone()
            };
            if configs.is_empty() {
                return emit_ok(args.format, "mcp", "tools", &McpToolsView { tools: vec![] });
            }
            let tools = connect_and_list(
                &configs,
                &profile.paths.project_root,
                &profile.paths.agent_id,
            )?;
            emit_ok(args.format, "mcp", "tools", &McpToolsView { tools })
        }
    }
}

fn env_pairs_from_map(map: &std::collections::HashMap<String, String>) -> Vec<String> {
    map.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

#[allow(clippy::too_many_arguments)]
fn build_server(
    name: String,
    command: Option<&str>,
    args: &[String],
    url: Option<&str>,
    env: &[String],
    headers: &[String],
    cwd: Option<std::path::PathBuf>,
    timeout_ms: Option<u64>,
) -> Result<McpServerConfig, ConfigCmdError> {
    let has_command = command.is_some_and(|c| !c.trim().is_empty());
    let has_url = url.is_some_and(|u| !u.trim().is_empty());
    if has_command == has_url {
        return Err(ConfigCmdError::message(
            "MCP server requires exactly one of `--command` (stdio) or `--url` (http)",
        ));
    }
    let timeout_ms = timeout_ms.unwrap_or_else(McpServerConfig::default_timeout_ms);
    if has_command {
        if !headers.is_empty() {
            return Err(ConfigCmdError::message(
                "stdio MCP servers must not set `--header` (use `--url` for HTTP)",
            ));
        }
        let env_map = parse_kv_pairs(env)?;
        let Some(command) = command.filter(|c| !c.trim().is_empty()) else {
            return Err(ConfigCmdError::message(
                "MCP server requires exactly one of `--command` (stdio) or `--url` (http)",
            ));
        };
        Ok(McpServerConfig {
            name,
            timeout_ms,
            transport: McpTransport::Stdio(McpStdioConfig {
                command: command.to_string(),
                args: args.to_vec(),
                env: env_map,
                cwd,
            }),
        })
    } else {
        if !args.is_empty() || !env.is_empty() || cwd.is_some() {
            return Err(ConfigCmdError::message(
                "HTTP MCP servers must not set `--arg`/`--env`/`--cwd`",
            ));
        }
        let headers_map = parse_kv_pairs(headers)?;
        let Some(url) = url.filter(|u| !u.trim().is_empty()) else {
            return Err(ConfigCmdError::message(
                "MCP server requires exactly one of `--command` (stdio) or `--url` (http)",
            ));
        };
        Ok(McpServerConfig {
            name,
            timeout_ms,
            transport: McpTransport::Http(McpHttpConfig {
                url: url.to_string(),
                headers: headers_map,
                auth: None,
            }),
        })
    }
}

fn server_view(server: &McpServerConfig) -> McpServerView {
    match &server.transport {
        McpTransport::Stdio(stdio) => McpServerView {
            name: server.name.clone(),
            transport: "stdio".to_string(),
            timeout_ms: server.timeout_ms,
            command: Some(stdio.command.clone()),
            args: Some(stdio.args.clone()),
            url: None,
            cwd: stdio.cwd.as_ref().map(|p| p.display().to_string()),
        },
        McpTransport::Http(http) => McpServerView {
            name: server.name.clone(),
            transport: "http".to_string(),
            timeout_ms: server.timeout_ms,
            command: None,
            args: None,
            url: Some(http.url.clone()),
            cwd: None,
        },
    }
}

fn connect_and_list(
    configs: &[McpServerConfig],
    project_root: &std::path::Path,
    agent_id: &str,
) -> Result<Vec<String>, ConfigCmdError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ConfigCmdError::from)?;
    runtime.block_on(async {
        let isolation = McpIsolationContext {
            project_root: Some(project_root.to_path_buf()),
            agent_id: agent_id.to_string(),
        };
        let registry =
            McpRegistry::connect_all_isolated(configs, None, CancellationToken::new(), isolation)
                .await
                .map_err(|err| ConfigCmdError::message(err.to_string()))?;
        let tools = registry.available_tools();
        registry.shutdown().await;
        Ok(tools)
    })
}
