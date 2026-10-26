//! `config skill` CRUD and tools mutations.

use std::io::{self, Read};

use serde::{Deserialize, Serialize};

use crate::access::QualifiedTool;
use crate::cli::{ConfigArgs, SkillBodyCmd, SkillCmd, SkillFileCmd, SkillToolsCmd};
use crate::settings::write_skills_source;
use crate::skills::dsl::{SkillDefinition, SkillsCatalog};
use crate::skills::package::{self, SkillPackage};

use super::common::{load_profile, parse_skills, read_cli_text};
use super::redact::validate_skill_fields;
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct SkillListEntry {
    name: String,
    policy: String,
    allowed_tools: Vec<String>,
    has_body: bool,
    file_count: usize,
}

#[derive(Debug, Serialize)]
struct SkillList {
    skills: Vec<SkillListEntry>,
}

#[derive(Debug, Serialize)]
struct SkillGet {
    name: String,
    policy: String,
    allowed_tools: Vec<String>,
    has_body: bool,
    file_count: usize,
    content: String,
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
                    skills: catalog
                        .skills
                        .iter()
                        .map(|skill| skill_list_entry(skill, &profile.settings.skill_packages))
                        .collect(),
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
            let package = profile.settings.skill_packages.get(&skill.name);
            emit_ok(
                args.format,
                "skill",
                "get",
                &SkillGet {
                    name: skill.name.clone(),
                    policy: skill.policy.clone(),
                    allowed_tools: skill.allowed_tools.clone(),
                    has_body: package.is_some(),
                    file_count: package.map(|item| item.files.len()).unwrap_or(0),
                    content: package.map(|item| item.body.clone()).unwrap_or_default(),
                },
            )
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
        SkillCmd::Replace => replace_catalog(args),
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
            package::remove_package_dir(&profile.paths.settings_dir, name)?;
            emit_ok(
                args.format,
                "skill",
                "remove",
                &serde_json::json!({ "name": name }),
            )
        }
        SkillCmd::Tools(sub) => run_tools(args, sub),
        SkillCmd::Body(sub) => run_body(args, sub),
        SkillCmd::File(sub) => run_file(args, sub),
    }
}

fn skill_list_entry(
    skill: &SkillDefinition,
    packages: &std::collections::BTreeMap<String, SkillPackage>,
) -> SkillListEntry {
    let package = packages.get(&skill.name);
    SkillListEntry {
        name: skill.name.clone(),
        policy: skill.policy.clone(),
        allowed_tools: skill.allowed_tools.clone(),
        has_body: package.is_some(),
        file_count: package.map(|item| item.files.len()).unwrap_or(0),
    }
}

fn require_skill<'a>(
    catalog: &'a SkillsCatalog,
    name: &str,
) -> Result<&'a SkillDefinition, ConfigCmdError> {
    catalog
        .skills
        .iter()
        .find(|skill| skill.name == name)
        .ok_or_else(|| ConfigCmdError::message(format!("skill `{name}` not found")))
}

#[derive(Debug, Deserialize)]
struct SkillReplaceEntry {
    name: String,
    policy: String,
    allowed_tools: Vec<String>,
    /// `None` leaves the skill folder untouched. Empty text clears `skill.md`.
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SkillReplaceBody {
    skills: Vec<SkillReplaceEntry>,
}

fn replace_catalog(args: &ConfigArgs) -> Result<(), ConfigCmdError> {
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    let body: SkillReplaceBody = serde_json::from_str(&raw)
        .map_err(|err| ConfigCmdError::message(format!("invalid skills JSON: {err}")))?;
    if body.skills.is_empty() {
        return Err(ConfigCmdError::message("at least one skill is required"));
    }
    for skill in &body.skills {
        validate_skill_fields(&skill.name, &skill.policy).map_err(ConfigCmdError::message)?;
        for tool in &skill.allowed_tools {
            QualifiedTool::parse(tool).map_err(ConfigCmdError::message)?;
        }
    }
    let profile = load_profile(&args.identity)?;
    let previous: Vec<String> = parse_skills(&profile.settings)?
        .skills
        .into_iter()
        .map(|skill| skill.name)
        .collect();
    let catalog = SkillsCatalog {
        skills: body
            .skills
            .iter()
            .map(|skill| SkillDefinition {
                name: skill.name.clone(),
                policy: skill.policy.clone(),
                allowed_tools: skill.allowed_tools.clone(),
            })
            .collect(),
    };
    save_catalog(&profile.paths.settings_dir, &catalog)?;
    for name in &previous {
        if !catalog.skills.iter().any(|skill| skill.name == *name) {
            package::remove_package_dir(&profile.paths.settings_dir, name)?;
        }
    }
    for skill in &body.skills {
        match skill.body.as_deref() {
            None => {}
            Some(content) if content.trim().is_empty() => {
                package::clear_body(&profile.paths.settings_dir, &skill.name)?;
            }
            Some(content) => {
                package::write_body(&profile.paths.settings_dir, &skill.name, content)?;
            }
        }
    }
    emit_ok(
        args.format,
        "skill",
        "replace",
        &serde_json::json!({ "ok": true, "skills": catalog.skills }),
    )
}

fn run_body(args: &ConfigArgs, cmd: &SkillBodyCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        SkillBodyCmd::Get { name } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, name)?;
            let content =
                package::read_body(&profile.paths.settings_dir, name)?.unwrap_or_default();
            emit_ok(
                args.format,
                "skill.body",
                "get",
                &serde_json::json!({ "name": name, "content": content }),
            )
        }
        SkillBodyCmd::Set {
            name,
            text,
            file,
            stdin,
        } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, name)?;
            let content = read_cli_text(text.as_deref(), file.as_deref(), *stdin, "skill body")?;
            package::write_body(&profile.paths.settings_dir, name, &content)?;
            emit_ok(
                args.format,
                "skill.body",
                "set",
                &serde_json::json!({ "name": name, "content": content }),
            )
        }
        SkillBodyCmd::Clear { name } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, name)?;
            package::clear_body(&profile.paths.settings_dir, name)?;
            emit_ok(
                args.format,
                "skill.body",
                "clear",
                &serde_json::json!({ "name": name, "content": "" }),
            )
        }
    }
}

fn run_file(args: &ConfigArgs, cmd: &SkillFileCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        SkillFileCmd::List { skill } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, skill)?;
            let files = package::list_package_files(&profile.paths.settings_dir, skill)?;
            emit_ok(
                args.format,
                "skill.file",
                "list",
                &serde_json::json!({ "skill": skill, "files": files }),
            )
        }
        SkillFileCmd::Get { skill, path } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, skill)?;
            let content = package::read_package_file(&profile.paths.settings_dir, skill, path)?;
            emit_ok(
                args.format,
                "skill.file",
                "get",
                &serde_json::json!({ "skill": skill, "path": path, "content": content }),
            )
        }
        SkillFileCmd::Put {
            skill,
            path,
            text,
            file,
            stdin,
        } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, skill)?;
            let content = read_cli_text(text.as_deref(), file.as_deref(), *stdin, "skill file")?;
            package::write_package_file(&profile.paths.settings_dir, skill, path, &content)?;
            emit_ok(
                args.format,
                "skill.file",
                "put",
                &serde_json::json!({ "skill": skill, "path": path }),
            )
        }
        SkillFileCmd::Remove { skill, path } => {
            let profile = load_profile(&args.identity)?;
            let catalog = parse_skills(&profile.settings)?;
            require_skill(&catalog, skill)?;
            package::remove_package_file(&profile.paths.settings_dir, skill, path)?;
            emit_ok(
                args.format,
                "skill.file",
                "remove",
                &serde_json::json!({ "skill": skill, "path": path }),
            )
        }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use std::fs;

    use crate::cli::{
        AgentIdentityArgs, ConfigArgs, ConfigFormat, InitArgs, SkillBodyCmd, SkillCmd, SkillFileCmd,
    };
    use crate::commands::init;
    use crate::project_paths::resolve_agent_identity;
    use crate::settings::load_settings;

    use super::run;

    #[test]
    fn skill_body_set_writes_skill_md() {
        let root = tempfile::tempdir().expect("temp");
        let agent_id = "body-demo";
        init::run(&InitArgs {
            agent_id: agent_id.to_string(),
            project_root: root.path().to_path_buf(),
            force: false,
            interactive: false,
        })
        .expect("init");
        let args = config_args(root.path(), agent_id);
        run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Set {
                name: "workspace".to_string(),
                text: Some("# Hello\n".to_string()),
                file: None,
                stdin: false,
            }),
        )
        .expect("set body");

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let settings = load_settings(&paths.settings_dir).expect("settings");
        let package = settings.skill_packages.get("workspace").expect("package");
        assert_eq!(package.body, "# Hello\n");
        assert_eq!(package.files.len(), 1);
    }

    #[test]
    fn skill_folder_commands_list_files_clear_and_remove() {
        let root = tempfile::tempdir().expect("temp");
        let agent_id = "folder-demo";
        init::run(&InitArgs {
            agent_id: agent_id.to_string(),
            project_root: root.path().to_path_buf(),
            force: false,
            interactive: false,
        })
        .expect("init");
        let args = config_args(root.path(), agent_id);
        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");

        let before_body = run(
            &args,
            &SkillCmd::File(SkillFileCmd::Put {
                skill: "workspace".to_string(),
                path: "scripts/example.py".to_string(),
                text: Some("print(1)\n".to_string()),
                file: None,
                stdin: false,
            }),
        );
        assert!(before_body.is_err(), "companion before skill.md");

        let missing_source = run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Set {
                name: "workspace".to_string(),
                text: None,
                file: None,
                stdin: false,
            }),
        );
        assert!(missing_source.is_err(), "set needs text, file, or stdin");

        let body_file = root.path().join("body.md");
        fs::write(&body_file, "# From file\n").expect("body file");
        run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Set {
                name: "workspace".to_string(),
                text: None,
                file: Some(body_file),
                stdin: false,
            }),
        )
        .expect("set from file");
        run(&args, &SkillCmd::List).expect("list");
        run(
            &args,
            &SkillCmd::Get {
                name: "workspace".to_string(),
            },
        )
        .expect("get");
        run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Get {
                name: "workspace".to_string(),
            }),
        )
        .expect("body get");

        run(
            &args,
            &SkillCmd::File(SkillFileCmd::Put {
                skill: "workspace".to_string(),
                path: "scripts/example.py".to_string(),
                text: Some("print(1)\n".to_string()),
                file: None,
                stdin: false,
            }),
        )
        .expect("put script");
        run(
            &args,
            &SkillCmd::File(SkillFileCmd::List {
                skill: "workspace".to_string(),
            }),
        )
        .expect("file list");
        run(
            &args,
            &SkillCmd::File(SkillFileCmd::Get {
                skill: "workspace".to_string(),
                path: "scripts/example.py".to_string(),
            }),
        )
        .expect("file get");
        assert!(run(
            &args,
            &SkillCmd::File(SkillFileCmd::Put {
                skill: "workspace".to_string(),
                path: "skill.md".to_string(),
                text: Some("no\n".to_string()),
                file: None,
                stdin: false,
            }),
        )
        .is_err());
        assert!(run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Clear {
                name: "workspace".to_string(),
            }),
        )
        .is_err());
        run(
            &args,
            &SkillCmd::File(SkillFileCmd::Remove {
                skill: "workspace".to_string(),
                path: "scripts/example.py".to_string(),
            }),
        )
        .expect("remove script");
        run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Clear {
                name: "workspace".to_string(),
            }),
        )
        .expect("clear");

        run(
            &args,
            &SkillCmd::Add {
                name: "notes".to_string(),
                policy: "short".to_string(),
                tools: vec!["home.read".to_string()],
            },
        )
        .expect("add");
        run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Get {
                name: "notes".to_string(),
            }),
        )
        .expect("empty body");
        crate::skills::package::write_body(&paths.settings_dir, "workspace", "# Again\n")
            .expect("rewrite");
        run(
            &args,
            &SkillCmd::Remove {
                name: "workspace".to_string(),
            },
        )
        .expect("remove");
        assert!(!paths.settings_dir.join("skills").join("workspace").exists());
        assert!(run(
            &args,
            &SkillCmd::Body(SkillBodyCmd::Get {
                name: "missing".to_string(),
            }),
        )
        .is_err());
        let last = run(
            &args,
            &SkillCmd::Remove {
                name: "notes".to_string(),
            },
        )
        .expect_err("last skill");
        assert!(last.to_string().contains("last skill"), "{last}");

        let extra = paths.settings_dir.join("skills").join("extra");
        fs::create_dir_all(&extra).expect("extra");
        fs::write(extra.join("skill.md"), "extra\n").expect("extra body");
        let undeclared = run(&args, &SkillCmd::List).expect_err("undeclared");
        assert!(undeclared.to_string().contains("extra"), "{undeclared}");
    }

    fn config_args(root: &Path, agent_id: &str) -> ConfigArgs {
        ConfigArgs {
            identity: AgentIdentityArgs {
                project_root: root.to_path_buf(),
                agent: agent_id.to_string(),
            },
            format: ConfigFormat::Json,
            command: crate::cli::ConfigCommand::Show,
        }
    }
}
