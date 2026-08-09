//! `config tools effective` — access ∩ skills [∩ discovered MCP].

use std::collections::{BTreeSet, HashSet};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::access::known_builtins;
use crate::cli::{ConfigArgs, ToolsCmd};
use crate::mcp::stdio_client::{McpIsolationContext, McpRegistry};
use crate::skills::dsl::SkillsCatalog;
use crate::tool_api::ToolExecutor;

use super::common::load_profile;
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct EffectiveTools {
    effective: Vec<String>,
    builtins: Vec<String>,
    skill_tools: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    configured_not_probed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    discovered_mcp: Option<Vec<String>>,
    connected: bool,
}

pub fn run(args: &ConfigArgs, cmd: &ToolsCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        ToolsCmd::Effective { connect } => {
            let profile = load_profile(&args.identity)?;
            let catalog = SkillsCatalog::parse(&profile.settings.skills_source)?;
            let skill_tools: BTreeSet<String> = catalog.allowed_tool_set().into_iter().collect();

            let access_builtins: BTreeSet<String> = profile
                .access
                .allowed_builtins()
                .iter()
                .map(|t| t.qualified())
                .collect();

            let builtin_names: HashSet<&str> = known_builtins().collect();
            let mut effective_builtins: Vec<String> = skill_tools
                .iter()
                .filter(|t| builtin_names.contains(t.as_str()) && access_builtins.contains(*t))
                .cloned()
                .collect();
            effective_builtins.sort();

            let mcp_from_skills: Vec<String> = skill_tools
                .iter()
                .filter(|t| !builtin_names.contains(t.as_str()))
                .cloned()
                .collect();

            let (effective_mcp, configured_not_probed, discovered) = if *connect {
                let discovered = if profile.config.mcp.is_empty() {
                    Vec::new()
                } else {
                    connect_discovered(
                        &profile.config.mcp,
                        &profile.paths.project_root,
                        &profile.paths.agent_id,
                    )?
                };
                let discovered_set: HashSet<_> = discovered.iter().cloned().collect();
                let mut effective: Vec<String> = mcp_from_skills
                    .iter()
                    .filter(|t| discovered_set.contains(*t))
                    .cloned()
                    .collect();
                effective.sort();
                (effective, Vec::new(), Some(discovered))
            } else {
                let mut not_probed = mcp_from_skills;
                not_probed.sort();
                (Vec::new(), not_probed, None)
            };

            let mut effective = effective_builtins.clone();
            effective.extend(effective_mcp);
            effective.sort();
            effective.dedup();

            let data = EffectiveTools {
                effective,
                builtins: effective_builtins,
                skill_tools: skill_tools.into_iter().collect(),
                configured_not_probed,
                discovered_mcp: discovered,
                connected: *connect,
            };
            emit_ok(args.format, "tools", "effective", &data)
        }
    }
}

fn connect_discovered(
    configs: &[crate::config::McpServerConfig],
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
