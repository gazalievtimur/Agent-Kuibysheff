//! `config access` get/set and builtins mutations.

use serde::Serialize;

use crate::access::{parse_tool_list, AccessPolicyConfig, QualifiedTool};
use crate::cli::{AccessCmd, BuiltinsCmd, ConfigArgs};
use crate::config::load_config;

use super::common::{load_profile, read_text_no_symlink, save_profile_config};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct BuiltinsList {
    builtins: Vec<String>,
}

pub fn run(args: &ConfigArgs, cmd: &AccessCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        AccessCmd::Get => {
            let profile = load_profile(&args.identity)?;
            emit_ok(args.format, "access", "get", &profile.config.access)
        }
        AccessCmd::Set { from_file } => {
            let raw = read_text_no_symlink(from_file)?;
            let access = parse_access_fragment(&raw, from_file)?;
            let mut profile = load_profile(&args.identity)?;
            profile.config.access = Some(access);
            save_profile_config(&profile.paths, &profile.config)?;
            // Re-resolve via load to ensure access policy compiles.
            let (cfg, _) = load_config(&profile.paths.config)?;
            emit_ok(args.format, "access", "set", &cfg.access)
        }
        AccessCmd::Builtins(sub) => run_builtins(args, sub),
    }
}

fn run_builtins(args: &ConfigArgs, cmd: &BuiltinsCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        BuiltinsCmd::List => {
            let profile = load_profile(&args.identity)?;
            let builtins = profile
                .config
                .access
                .as_ref()
                .map(|a| a.tools.builtins.clone())
                .unwrap_or_default();
            emit_ok(
                args.format,
                "access.builtins",
                "list",
                &BuiltinsList { builtins },
            )
        }
        BuiltinsCmd::Add { tool } => {
            let qt = QualifiedTool::parse(tool).map_err(ConfigCmdError::message)?;
            let mut profile = load_profile(&args.identity)?;
            let access = profile
                .config
                .access
                .get_or_insert_with(AccessPolicyConfig::default);
            let name = qt.qualified();
            if !access.tools.builtins.iter().any(|t| t == &name) {
                access.tools.builtins.push(name.clone());
            }
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(
                args.format,
                "access.builtins",
                "add",
                &BuiltinsList {
                    builtins: profile
                        .config
                        .access
                        .as_ref()
                        .map(|a| a.tools.builtins.clone())
                        .unwrap_or_default(),
                },
            )
        }
        BuiltinsCmd::Remove { tool } => {
            let qt = QualifiedTool::parse(tool).map_err(ConfigCmdError::message)?;
            let name = qt.qualified();
            let mut profile = load_profile(&args.identity)?;
            let Some(access) = profile.config.access.as_mut() else {
                return Err(ConfigCmdError::message(
                    "no `access` section configured; cannot remove builtins",
                ));
            };
            let before = access.tools.builtins.len();
            access.tools.builtins.retain(|t| t != &name);
            if access.tools.builtins.len() == before {
                return Err(ConfigCmdError::message(format!(
                    "builtin `{name}` was not in the allowlist"
                )));
            }
            let builtins = access.tools.builtins.clone();
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(
                args.format,
                "access.builtins",
                "remove",
                &BuiltinsList { builtins },
            )
        }
    }
}

fn parse_access_fragment(
    raw: &str,
    path: &std::path::Path,
) -> Result<AccessPolicyConfig, ConfigCmdError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let parsed = match extension.as_str() {
        "json" => serde_json::from_str::<AccessPolicyConfig>(raw)
            .map_err(|err| ConfigCmdError::message(format!("failed to parse access JSON: {err}"))),
        _ => serde_yaml::from_str::<AccessPolicyConfig>(raw)
            .or_else(|_| serde_json::from_str::<AccessPolicyConfig>(raw))
            .map_err(|err| ConfigCmdError::message(format!("failed to parse access file: {err}"))),
    }?;
    // Soft-touch: ensure builtins parse as qualified tools when present.
    if !parsed.tools.builtins.is_empty() {
        parse_tool_list(&parsed.tools.builtins).map_err(ConfigCmdError::message)?;
    }
    Ok(parsed)
}
