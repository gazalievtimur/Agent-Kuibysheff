//! `kbshff mcp discover` / `kbshff mcp call`: connect configured MCP servers.

use std::io::{self, Read};
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::cli::{ConfigFormat, McpProbeArgs, McpProbeCommand};

use crate::config::{load_config, McpServerConfig};
use crate::mcp::{McpIsolationContext, McpRegistry, McpToolDescriptor};
use crate::project_paths::resolve_agent_identity;

/// One MCP server after a studio discover attempt.
#[derive(Debug, Serialize)]
pub struct McpServerProbe {
    pub name: String,
    pub tools: Vec<McpToolDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of discovering tools across configured MCP servers.
#[derive(Debug, Serialize)]
pub struct McpDiscoverResult {
    pub servers: Vec<McpServerProbe>,
}

/// Successful `tools/call` from the studio probe.
#[derive(Debug, Serialize)]
pub struct McpCallResult {
    pub server: String,
    pub tool: String,
    pub result: Value,
}

fn isolation(project_root: &Path, agent_id: &str) -> McpIsolationContext {
    McpIsolationContext {
        project_root: Some(project_root.to_path_buf()),
        agent_id: agent_id.to_string(),
    }
}

fn load_mcp_configs(project_root: &Path, agent_id: &str) -> Result<Vec<McpServerConfig>, String> {
    let paths = resolve_agent_identity(project_root, agent_id, None).map_err(|e| e.to_string())?;
    let (cfg, _) = load_config(&paths.config).map_err(|e| e.to_string())?;
    Ok(cfg.mcp)
}

/// Connect each configured MCP server (or one named server) and list tool schemas.
///
/// Servers are probed independently so one failure does not hide the others.
pub async fn discover(
    project_root: &Path,
    agent_id: &str,
    server_filter: Option<&str>,
) -> Result<McpDiscoverResult, String> {
    let configs = load_mcp_configs(project_root, agent_id)?;
    if configs.is_empty() {
        return Ok(McpDiscoverResult {
            servers: Vec::new(),
        });
    }

    let targets: Vec<&McpServerConfig> = match server_filter {
        Some(name) => {
            let found: Vec<_> = configs.iter().filter(|c| c.name == name).collect();
            if found.is_empty() {
                return Err(format!(
                    "MCP server `{name}` is not in the saved agent config"
                ));
            }
            found
        }
        None => configs.iter().collect(),
    };

    let iso = isolation(project_root, agent_id);
    let mut servers = Vec::with_capacity(targets.len());

    for cfg in targets {
        let cancel = CancellationToken::new();
        match McpRegistry::connect_all_isolated(
            std::slice::from_ref(cfg),
            None,
            cancel,
            iso.clone(),
        )
        .await
        {
            Ok(registry) => {
                let tools = match registry.list_tool_descriptors().await {
                    Ok(t) => t,
                    Err(e) => {
                        registry.shutdown().await;
                        servers.push(McpServerProbe {
                            name: cfg.name.clone(),
                            tools: Vec::new(),
                            error: Some(e.to_string()),
                        });
                        continue;
                    }
                };
                registry.shutdown().await;
                servers.push(McpServerProbe {
                    name: cfg.name.clone(),
                    tools,
                    error: None,
                });
            }
            Err(e) => {
                servers.push(McpServerProbe {
                    name: cfg.name.clone(),
                    tools: Vec::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(McpDiscoverResult { servers })
}

/// Connect the named MCP server and invoke one tool with the given arguments.
pub async fn call_tool(
    project_root: &Path,
    agent_id: &str,
    server: &str,
    tool: &str,
    arguments: Value,
) -> Result<McpCallResult, String> {
    let configs = load_mcp_configs(project_root, agent_id)?;
    let cfg = configs
        .iter()
        .find(|c| c.name == server)
        .ok_or_else(|| format!("MCP server `{server}` is not in the saved agent config"))?
        .clone();

    let cancel = CancellationToken::new();
    let registry =
        McpRegistry::connect_all_isolated(&[cfg], None, cancel, isolation(project_root, agent_id))
            .await
            .map_err(|e| e.to_string())?;

    let result = registry.call_discovered_tool(server, tool, arguments).await;
    registry.shutdown().await;

    let result = result.map_err(|e| e.to_string())?;
    Ok(McpCallResult {
        server: server.to_string(),
        tool: tool.to_string(),
        result,
    })
}

/// Run `kbshff mcp`.
///
/// # Errors
///
/// Returns a message when the runtime, saved config, or MCP call fails.
pub fn run(args: &McpProbeArgs) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to start tokio runtime: {err}"))?;
    runtime.block_on(run_cli(args))
}

async fn run_cli(args: &McpProbeArgs) -> Result<(), String> {
    match &args.command {
        McpProbeCommand::Discover { server } => {
            let result = discover(
                &args.identity.project_root,
                &args.identity.agent,
                server.as_deref(),
            )
            .await?;
            emit_discover(args.format, &result)
        }
        McpProbeCommand::Call { server, tool } => {
            if server.trim().is_empty() || tool.trim().is_empty() {
                return Err("server and tool are required".to_string());
            }
            let arguments = read_call_arguments()?;
            let result = call_tool(
                &args.identity.project_root,
                &args.identity.agent,
                server,
                tool,
                arguments,
            )
            .await?;
            emit_call(args.format, &result)
        }
    }
}

fn read_call_arguments() -> Result<Value, String> {
    let mut raw = String::new();
    io::stdin()
        .read_to_string(&mut raw)
        .map_err(|err| err.to_string())?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed).map_err(|err| format!("invalid tool arguments JSON: {err}"))
}

fn emit_discover(format: ConfigFormat, result: &McpDiscoverResult) -> Result<(), String> {
    match format {
        ConfigFormat::Json => crate::commands::print_json(result).map_err(|err| err.to_string()),
        ConfigFormat::Text => {
            for server in &result.servers {
                if let Some(error) = &server.error {
                    println!("{} error: {error}", server.name);
                    continue;
                }
                if server.tools.is_empty() {
                    println!("{} (no tools)", server.name);
                }
                for tool in &server.tools {
                    println!("{}.{}", server.name, tool.name);
                }
            }
            Ok(())
        }
    }
}

fn emit_call(format: ConfigFormat, result: &McpCallResult) -> Result<(), String> {
    match format {
        ConfigFormat::Json => crate::commands::print_json(result).map_err(|err| err.to_string()),
        ConfigFormat::Text => {
            println!("{}.{}", result.server, result.tool);
            println!(
                "{}",
                serde_json::to_string_pretty(&result.result).unwrap_or_else(|_| "{}".to_string())
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InitArgs;
    use crate::commands::init;

    #[tokio::test]
    async fn discover_with_no_mcp_servers_is_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init::run(&InitArgs {
            agent_id: "demo".to_string(),
            project_root: tmp.path().to_path_buf(),
            force: true,
            interactive: false,
        })
        .expect("init");
        let result = discover(tmp.path(), "demo", None).await.expect("discover");
        assert!(result.servers.is_empty());
    }
}
