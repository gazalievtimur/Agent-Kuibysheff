//! `config skill` CRUD and tools mutations.

use serde::Serialize;

use crate::access::QualifiedTool;
use crate::cli::{ConfigArgs, SkillCmd, SkillToolsCmd};
use crate::settings::write_skills_source;
use crate::skills::dsl::{SkillDefinition, SkillsCatalog};

use super::common::{load_profile, parse_skills};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct SkillList {
    skills: Vec<SkillDefinition>,
}

pub fn run(args: &ConfigArgs, cmd: &SkillCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        SkillCmd::List => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            emit_ok(
                args.format,
                "skill",
                "list",
                &SkillList {
                    skills: catalog.skills,
                },
            )
        }
        SkillCmd::Get { name } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            let skill = catalog
                .skills
                .iter()
                .find(|s| s.name == *name)
                .ok_or_else(|| ConfigCmdError::message(format!("skill `{name}` not found")))?;
            emit_ok(args.format, "skill", "get", skill)
        }
        SkillCmd::Add {
            name,
            policy,
            tools,
        } => {
            let profile = load_profile(&args.identity)?;
            let mut catalog = parse_skills(&profile.settings)?;
            if catalog.skills.iter().any(|s| s.name == *name) {
                return Err(ConfigCmdError::message(format!(
                    "skill `{name}` already exists"
                )));
            }
            let allowed_tools = normalize_tools(tools)?;
            catalog.skills.push(SkillDefinition {
                name: name.clone(),
                policy: policy.clone(),
                allowed_tools,
            });
            save_catalog(&profile.paths.settings_dir, &catalog)?;
            let skill = catalog.skills.last().expect("just pushed");
            emit_ok(args.format, "skill", "add", skill)
        }
        SkillCmd::Set {
            name,
            policy,
            tools,
        } => {
            let profile = load_profile(&args.identity)?;
            let mut catalog = parse_skills(&profile.settings)?;
            let skill = catalog
                .skills
                .iter_mut()
                .find(|s| s.name == *name)
                .ok_or_else(|| ConfigCmdError::message(format!("skill `{name}` not found")))?;
            if let Some(p) = policy {
                skill.policy = p.clone();
            }
            if !tools.is_empty() {
                skill.allowed_tools = normalize_tools(tools)?;
            }
            if policy.is_none() && tools.is_empty() {
                return Err(ConfigCmdError::message(
                    "skill set requires `--policy` and/or `--tool`",
                ));
            }
            let snapshot = skill.clone();
            save_catalog(&profile.paths.settings_dir, &catalog)?;
            emit_ok(args.format, "skill", "set", &snapshot)
        }
        SkillCmd::Remove { name } => {
            let profile = load_profile(&args.identity)?;
            let mut catalog = parse_skills(&profile.settings)?;
            let before = catalog.skills.len();
            catalog.skills.retain(|s| s.name != *name);
            if catalog.skills.len() == before {
                return Err(ConfigCmdError::message(format!("skill `{name}` not found")));
            }
            if catalog.skills.is_empty() {
                return Err(ConfigCmdError::message(
                    "refusing to remove the last skill; at least one skill is required",
                ));
            }
            save_catalog(&profile.paths.settings_dir, &catalog)?;
            emit_ok(
                args.format,
                "skill",
                "remove",
                &serde_json::json!({ "name": name }),
            )
        }
        SkillCmd::Tools(sub) => run_tools(args, sub),
    }
}

fn run_tools(args: &ConfigArgs, cmd: &SkillToolsCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        SkillToolsCmd::Add { skill, tools } => {
            let profile = load_profile(&args.identity)?;
            let mut catalog = parse_skills(&profile.settings)?;
            let entry = catalog
                .skills
                .iter_mut()
                .find(|s| s.name == *skill)
                .ok_or_else(|| ConfigCmdError::message(format!("skill `{skill}` not found")))?;
            for tool in normalize_tools(tools)? {
                if !entry.allowed_tools.iter().any(|t| t == &tool) {
                    entry.allowed_tools.push(tool);
                }
            }
            let snapshot = entry.clone();
            save_catalog(&profile.paths.settings_dir, &catalog)?;
            emit_ok(args.format, "skill.tools", "add", &snapshot)
        }
        SkillToolsCmd::Remove { skill, tools } => {
            let profile = load_profile(&args.identity)?;
            let mut catalog = parse_skills(&profile.settings)?;
            let entry = catalog
                .skills
                .iter_mut()
                .find(|s| s.name == *skill)
                .ok_or_else(|| ConfigCmdError::message(format!("skill `{skill}` not found")))?;
            let remove_set = normalize_tools(tools)?;
            entry
                .allowed_tools
                .retain(|t| !remove_set.iter().any(|r| r == t));
            if entry.allowed_tools.is_empty() {
                return Err(ConfigCmdError::message(format!(
                    "skill `{skill}` would have empty allowed_tools"
                )));
            }
            let snapshot = entry.clone();
            save_catalog(&profile.paths.settings_dir, &catalog)?;
            emit_ok(args.format, "skill.tools", "remove", &snapshot)
        }
    }
}

fn normalize_tools(tools: &[String]) -> Result<Vec<String>, ConfigCmdError> {
    if tools.is_empty() {
        return Err(ConfigCmdError::message(
            "at least one qualified tool (`server.tool`) is required",
        ));
    }
    let mut out = Vec::with_capacity(tools.len());
    for raw in tools {
        let qt = QualifiedTool::parse(raw).map_err(ConfigCmdError::message)?;
        let q = qt.qualified();
        if !out.iter().any(|t| t == &q) {
            out.push(q);
        }
    }
    Ok(out)
}

fn save_catalog(
    settings_dir: &std::path::Path,
    catalog: &SkillsCatalog,
) -> Result<(), ConfigCmdError> {
    if catalog.skills.is_empty() {
        return Err(ConfigCmdError::message("skills catalog must not be empty"));
    }
    let rendered = catalog.render();
    SkillsCatalog::parse(&rendered)?;
    write_skills_source(settings_dir, &rendered)?;
    Ok(())
}
