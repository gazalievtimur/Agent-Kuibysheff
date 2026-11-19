//! Saved-profile configuration tools.
//!
//! Writes go to the agent profile on disk and are visible on the next start.
//! The running process keeps the policy, prompt, and MCP connections it loaded
//! at startup. Secrets, the running agent's provider URL, and MCP env/headers
//! are not accepted and are not returned.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::access::{AccessPolicyConfig, QualifiedTool};
use crate::billing::Money;
use crate::cli::InitArgs;
use crate::commands::init;
use crate::config::{
    load_config, save_config, AppConfig, McpHttpConfig, McpServerConfig, McpStdioConfig,
    McpTransport,
};
use crate::project_paths::{resolve_agent_identity, ResolvedAgentPaths};
use crate::settings::{load_settings, write_master_prompt, write_rules, AgentSettings};
use crate::skills::dsl::{SkillDefinition, SkillsCatalog};
use crate::skills::package::{self, catalog_from_settings};
use crate::tool_api::{ExternalToolError, ToolError};
use crate::tools::registry::is_known_builtin;

/// Same order of magnitude as the character window used by file reads.
const MAX_PROFILE_TEXT_CHARS: usize = 200_000;

/// Disk editor for profiles under one project root.
#[derive(Debug, Clone)]
pub struct ConfigEditor {
    project_root: Option<PathBuf>,
    running_agent: String,
}

struct Profile {
    paths: ResolvedAgentPaths,
    config: AppConfig,
    settings: AgentSettings,
}

impl ConfigEditor {
    #[must_use]
    pub fn new(project_root: Option<PathBuf>, running_agent: impl Into<String>) -> Self {
        Self {
            project_root,
            running_agent: running_agent.into(),
        }
    }

    /// Dispatches one `config.*` tool.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] for unknown tools, invalid arguments, or a rejected edit.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, ToolError> {
        let editor = self.clone();
        let tool = tool.to_string();
        tokio::task::spawn_blocking(move || editor.call_sync(&tool, arguments))
            .await
            .map_err(|error| failed(error.to_string()))?
    }

    fn call_sync(&self, tool: &str, arguments: Value) -> Result<Value, ToolError> {
        match tool {
            "show" => self.show(decode(tool, arguments)?),
            "rules_get" => self.rules_get(decode(tool, arguments)?),
            "rules_set" => self.rules_set(decode(tool, arguments)?),
            "prompt_get" => self.prompt_get(decode(tool, arguments)?),
            "prompt_set" => self.prompt_set(decode(tool, arguments)?),
            "skill_list" => self.skill_list(decode(tool, arguments)?),
            "skill_get" => self.skill_get(decode(tool, arguments)?),
            "skill_add" => self.skill_add(decode(tool, arguments)?),
            "skill_set" => self.skill_set(decode(tool, arguments)?),
            "skill_remove" => self.skill_remove(decode(tool, arguments)?),
            "skill_body_get" => self.skill_body_get(decode(tool, arguments)?),
            "skill_body_set" => self.skill_body_set(decode(tool, arguments)?),
            "skill_file_list" => self.skill_file_list(decode(tool, arguments)?),
            "skill_file_get" => self.skill_file_get(decode(tool, arguments)?),
            "skill_file_put" => self.skill_file_put(decode(tool, arguments)?),
            "skill_file_remove" => self.skill_file_remove(decode(tool, arguments)?),
            "mcp_list" => self.mcp_list(decode(tool, arguments)?),
            "mcp_get" => self.mcp_get(decode(tool, arguments)?),
            "mcp_add" => self.mcp_add(decode(tool, arguments)?),
            "mcp_set" => self.mcp_set(decode(tool, arguments)?),
            "mcp_remove" => self.mcp_remove(decode(tool, arguments)?),
            "limits_get" => self.limits_get(decode(tool, arguments)?),
            "limits_set" => self.limits_set(decode(tool, arguments)?),
            "provider_get" => self.provider_get(decode(tool, arguments)?),
            "provider_set" => self.provider_set(decode(tool, arguments)?),
            "access_get" => self.access_get(decode(tool, arguments)?),
            "builtins_add" => self.builtins_add(decode(tool, arguments)?),
            "builtins_remove" => self.builtins_remove(decode(tool, arguments)?),
            "init" => self.init_profile(decode(tool, arguments)?),
            other => Err(ToolError::External(ExternalToolError::UnknownTool {
                server: "config".to_string(),
                tool: other.to_string(),
            })),
        }
    }

    fn show(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let skills = parse_skills(&profile)?;
        let builtins = profile
            .config
            .access
            .as_ref()
            .map(|access| access.tools.builtins.clone())
            .unwrap_or_default();
        Ok(json!({
            "agent": profile.paths.agent_id,
            "running_agent": self.running_agent,
            "edits_apply": "next_start",
            "provider": provider_view(&profile.config),
            "limits": limits_view(&profile.config),
            "mcp_servers": profile.config.mcp.iter().map(mcp_public_view).collect::<Vec<_>>(),
            "skills": skills.skills.iter().map(|skill| skill.name.clone()).collect::<Vec<_>>(),
            "has_rules": !profile.settings.rules.trim().is_empty(),
            "builtins": builtins,
        }))
    }

    fn rules_get(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        Ok(text_view("rules", &profile.settings.rules))
    }

    fn rules_set(&self, args: ContentArgs) -> Result<Value, ToolError> {
        check_text("rules", &args.content)?;
        let profile = self.load(&args.agent)?;
        write_rules(&profile.paths.settings_dir, &args.content).map_err(failed)?;
        Ok(saved(text_view("rules", &args.content)))
    }

    fn prompt_get(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        Ok(text_view("master_prompt", &profile.settings.master_prompt))
    }

    fn prompt_set(&self, args: ContentArgs) -> Result<Value, ToolError> {
        check_text("master_prompt", &args.content)?;
        let profile = self.load(&args.agent)?;
        write_master_prompt(&profile.paths.settings_dir, &args.content).map_err(failed)?;
        Ok(saved(text_view("master_prompt", &args.content)))
    }

    fn skill_list(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        let skills: Vec<Value> = catalog
            .skills
            .iter()
            .map(|skill| skill_view(skill, &profile))
            .collect();
        Ok(json!({ "skills": skills }))
    }

    fn skill_get(&self, args: NameArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        let skill = require_skill(&catalog, &args.name)?;
        let mut view = skill_view(skill, &profile);
        let body = package::read_body(&profile.paths.settings_dir, &skill.name).map_err(failed)?;
        if let Some(body) = body {
            let (content, truncated) = bound_text(&body);
            view["content"] = json!(content);
            view["truncated"] = json!(truncated);
        }
        Ok(view)
    }

    fn skill_add(&self, args: SkillAddArgs) -> Result<Value, ToolError> {
        validate_skill_fields(&args.name, &args.policy)?;
        package::validate_skill_dir_name(&args.name).map_err(invalid)?;
        let tools = normalize_tools(&args.tools)?;
        let mut profile = self.load(&args.agent)?;
        let mut catalog = parse_skills(&profile)?;
        if catalog.skills.iter().any(|skill| skill.name == args.name) {
            return Err(invalid(format!("skill `{}` already exists", args.name)));
        }
        catalog.skills.push(SkillDefinition {
            name: args.name.clone(),
            policy: args.policy.clone(),
            allowed_tools: tools,
        });
        save_catalog(&profile.paths.settings_dir, &catalog)?;
        profile.settings = load_settings(&profile.paths.settings_dir).map_err(failed)?;
        let skill = catalog.skills.last().expect("just pushed");
        Ok(saved(skill_view(skill, &profile)))
    }

    fn skill_set(&self, args: SkillSetArgs) -> Result<Value, ToolError> {
        if args.policy.is_none() && args.tools.is_none() {
            return Err(invalid(
                "skill_set requires policy and/or tools".to_string(),
            ));
        }
        let mut profile = self.load(&args.agent)?;
        let mut catalog = parse_skills(&profile)?;
        let skill = catalog
            .skills
            .iter_mut()
            .find(|skill| skill.name == args.name)
            .ok_or_else(|| invalid(format!("skill `{}` not found", args.name)))?;
        if let Some(policy) = args.policy {
            validate_skill_fields(&args.name, &policy)?;
            skill.policy = policy;
        }
        if let Some(tools) = args.tools {
            skill.allowed_tools = normalize_tools(&tools)?;
        }
        let snapshot = skill.clone();
        save_catalog(&profile.paths.settings_dir, &catalog)?;
        profile.settings = load_settings(&profile.paths.settings_dir).map_err(failed)?;
        Ok(saved(skill_view(&snapshot, &profile)))
    }

    fn skill_remove(&self, args: NameArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let mut catalog = parse_skills(&profile)?;
        let before = catalog.skills.len();
        catalog.skills.retain(|skill| skill.name != args.name);
        if catalog.skills.len() == before {
            return Err(invalid(format!("skill `{}` not found", args.name)));
        }
        if catalog.skills.is_empty() {
            return Err(invalid(
                "refusing to remove the last skill; at least one skill is required".to_string(),
            ));
        }
        save_catalog(&profile.paths.settings_dir, &catalog)?;
        package::remove_package_dir(&profile.paths.settings_dir, &args.name).map_err(failed)?;
        Ok(saved(json!({ "name": args.name })))
    }

    fn skill_body_get(&self, args: NameArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        require_skill(&catalog, &args.name)?;
        let body = package::read_body(&profile.paths.settings_dir, &args.name)
            .map_err(failed)?
            .unwrap_or_default();
        Ok(text_view("content", &body))
    }

    fn skill_body_set(&self, args: NameContentArgs) -> Result<Value, ToolError> {
        check_text("skill.md", &args.content)?;
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        require_skill(&catalog, &args.name)?;
        package::write_body(&profile.paths.settings_dir, &args.name, &args.content)
            .map_err(failed)?;
        Ok(saved(text_view("content", &args.content)))
    }

    fn skill_file_list(&self, args: NameArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        require_skill(&catalog, &args.name)?;
        let files =
            package::list_package_files(&profile.paths.settings_dir, &args.name).map_err(failed)?;
        Ok(json!({ "name": args.name, "files": files }))
    }

    fn skill_file_get(&self, args: FileArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        require_skill(&catalog, &args.name)?;
        let content =
            package::read_package_file(&profile.paths.settings_dir, &args.name, &args.path)
                .map_err(failed)?;
        let (content, truncated) = bound_text(&content);
        Ok(json!({
            "name": args.name,
            "path": args.path,
            "content": content,
            "truncated": truncated,
        }))
    }

    fn skill_file_put(&self, args: FilePutArgs) -> Result<Value, ToolError> {
        check_text("skill file", &args.content)?;
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        require_skill(&catalog, &args.name)?;
        package::write_package_file(
            &profile.paths.settings_dir,
            &args.name,
            &args.path,
            &args.content,
        )
        .map_err(failed)?;
        Ok(saved(json!({
            "name": args.name,
            "path": args.path,
        })))
    }

    fn skill_file_remove(&self, args: FileArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let catalog = parse_skills(&profile)?;
        require_skill(&catalog, &args.name)?;
        package::remove_package_file(&profile.paths.settings_dir, &args.name, &args.path)
            .map_err(failed)?;
        Ok(saved(json!({ "name": args.name, "path": args.path })))
    }

    fn mcp_list(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        Ok(json!({
            "servers": profile.config.mcp.iter().map(mcp_public_view).collect::<Vec<_>>(),
        }))
    }

    fn mcp_get(&self, args: NameArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        let server = find_mcp(&profile.config, &args.name)?;
        Ok(mcp_public_view(server))
    }

    fn mcp_add(&self, args: McpAddArgs) -> Result<Value, ToolError> {
        let server = build_mcp_add(&args)?;
        let mut profile = self.load(&args.agent)?;
        if profile
            .config
            .mcp
            .iter()
            .any(|item| item.name == server.name)
        {
            return Err(invalid(format!(
                "MCP server `{}` already exists",
                server.name
            )));
        }
        profile.config.mcp.push(server);
        self.save_config(&profile)?;
        let view = mcp_public_view(profile.config.mcp.last().expect("just pushed"));
        Ok(saved(view))
    }

    fn mcp_set(&self, args: McpSetArgs) -> Result<Value, ToolError> {
        let mut profile = self.load(&args.agent)?;
        let idx = profile
            .config
            .mcp
            .iter()
            .position(|server| server.name == args.name)
            .ok_or_else(|| invalid(format!("MCP server `{}` not found", args.name)))?;
        let updated = build_mcp_set(&profile.config.mcp[idx], &args)?;
        profile.config.mcp[idx] = updated;
        self.save_config(&profile)?;
        Ok(saved(mcp_public_view(&profile.config.mcp[idx])))
    }

    fn mcp_remove(&self, args: NameArgs) -> Result<Value, ToolError> {
        let mut profile = self.load(&args.agent)?;
        let before = profile.config.mcp.len();
        profile.config.mcp.retain(|server| server.name != args.name);
        if profile.config.mcp.len() == before {
            return Err(invalid(format!("MCP server `{}` not found", args.name)));
        }
        self.save_config(&profile)?;
        Ok(saved(json!({ "name": args.name })))
    }

    fn limits_get(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        Ok(limits_view(&profile.config))
    }

    fn limits_set(&self, args: LimitsArgs) -> Result<Value, ToolError> {
        if args.max_iterations.is_none()
            && args.max_tokens.is_none()
            && args.max_duration_sec.is_none()
            && args.max_cost.is_none()
        {
            return Err(invalid(
                "limits_set requires at least one field".to_string(),
            ));
        }
        let mut profile = self.load(&args.agent)?;
        if let Some(value) = args.max_iterations {
            profile.config.limits.max_iterations = value;
        }
        if let Some(value) = args.max_tokens {
            profile.config.limits.max_tokens = value;
        }
        if let Some(value) = args.max_duration_sec {
            profile.config.limits.max_duration_sec = value;
        }
        if let Some(raw) = args.max_cost {
            let money = Money::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
            profile.config.limits.max_cost = Some(money);
        }
        self.save_config(&profile)?;
        Ok(saved(limits_view(&profile.config)))
    }

    fn provider_get(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        Ok(provider_view(&profile.config))
    }

    fn provider_set(&self, args: ProviderArgs) -> Result<Value, ToolError> {
        if args.base_url.is_none()
            && args.model.is_none()
            && args.timeout_ms.is_none()
            && args.max_retries.is_none()
            && args.effort.is_none()
            && args.thinking.is_none()
        {
            return Err(invalid(
                "provider_set requires at least one field".to_string(),
            ));
        }
        let mut profile = self.load(&args.agent)?;
        if let Some(base_url) = args.base_url {
            if profile.paths.agent_id == self.running_agent {
                return Err(invalid(
                    "provider.base_url of the running agent cannot be changed".to_string(),
                ));
            }
            if base_url.trim().is_empty() {
                return Err(invalid("provider.base_url must not be empty".to_string()));
            }
            profile.config.provider.base_url = base_url;
        }
        if let Some(model) = args.model {
            if model.trim().is_empty() {
                return Err(invalid("provider.model must not be empty".to_string()));
            }
            profile.config.provider.model = model;
        }
        if let Some(timeout_ms) = args.timeout_ms {
            profile.config.provider.timeout_ms = timeout_ms;
        }
        if let Some(max_retries) = args.max_retries {
            profile.config.provider.max_retries = max_retries;
        }
        if let Some(effort) = args.effort {
            let trimmed = effort.trim();
            if trimmed.is_empty() {
                return Err(invalid("provider.effort must not be empty".to_string()));
            }
            profile.config.provider.effort = Some(trimmed.to_string());
        }
        if let Some(thinking) = args.thinking {
            profile.config.provider.thinking = Some(thinking);
        }
        self.save_config(&profile)?;
        Ok(saved(provider_view(&profile.config)))
    }

    fn access_get(&self, args: AgentArgs) -> Result<Value, ToolError> {
        let profile = self.load(&args.agent)?;
        Ok(json!({ "access": profile.config.access }))
    }

    fn builtins_add(&self, args: ToolArgs) -> Result<Value, ToolError> {
        let name = known_builtin_name(&args.tool)?;
        let mut profile = self.load(&args.agent)?;
        let access = profile
            .config
            .access
            .get_or_insert_with(AccessPolicyConfig::default);
        if !access.tools.builtins.iter().any(|tool| tool == &name) {
            access.tools.builtins.push(name);
        }
        let builtins = access.tools.builtins.clone();
        self.save_config(&profile)?;
        Ok(saved(json!({ "builtins": builtins })))
    }

    fn builtins_remove(&self, args: ToolArgs) -> Result<Value, ToolError> {
        let name = QualifiedTool::parse(&args.tool)
            .map_err(invalid)?
            .qualified();
        let mut profile = self.load(&args.agent)?;
        let Some(access) = profile.config.access.as_mut() else {
            return Err(invalid(
                "no access section configured; cannot remove builtins".to_string(),
            ));
        };
        let before = access.tools.builtins.len();
        access.tools.builtins.retain(|tool| tool != &name);
        if access.tools.builtins.len() == before {
            return Err(invalid(format!(
                "builtin `{name}` was not in the allowlist"
            )));
        }
        let builtins = access.tools.builtins.clone();
        self.save_config(&profile)?;
        Ok(saved(json!({ "builtins": builtins })))
    }

    fn init_profile(&self, args: InitToolArgs) -> Result<Value, ToolError> {
        let project = self.project()?;
        let agent = require_id(&args.agent)?;
        let result = init::run(&InitArgs {
            agent_id: agent,
            project_root: project.to_path_buf(),
            force: false,
            interactive: false,
        })
        .map_err(failed)?;
        let files: Vec<String> = result
            .written_files
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        Ok(saved(json!({
            "agent": result.agent_id,
            "files": files,
        })))
    }

    fn load(&self, agent: &Option<String>) -> Result<Profile, ToolError> {
        let project = self.project()?;
        let agent = self.target_id(agent)?;
        let paths = resolve_agent_identity(project, &agent, None).map_err(failed)?;
        let (config, _) = load_config(&paths.config).map_err(failed)?;
        let settings = load_settings(&paths.settings_dir).map_err(failed)?;
        Ok(Profile {
            paths,
            config,
            settings,
        })
    }

    fn save_config(&self, profile: &Profile) -> Result<(), ToolError> {
        save_config(&profile.paths.config, &profile.config).map_err(failed)
    }

    fn project(&self) -> Result<&Path, ToolError> {
        self.project_root
            .as_deref()
            .ok_or_else(|| invalid("this run has no project root".to_string()))
    }

    fn target_id(&self, agent: &Option<String>) -> Result<String, ToolError> {
        match agent {
            Some(id) => require_id(id),
            None => require_id(&self.running_agent),
        }
    }
}

fn require_id(agent: &str) -> Result<String, ToolError> {
    let id = agent.trim();
    if id.is_empty() {
        return Err(invalid("agent id must not be empty".to_string()));
    }
    crate::project_paths::validate_agent_id(id).map_err(failed)?;
    Ok(id.to_string())
}

fn parse_skills(profile: &Profile) -> Result<SkillsCatalog, ToolError> {
    catalog_from_settings(
        &profile.settings.skills_source,
        &profile.settings.skill_packages,
    )
    .map_err(failed)
}

fn require_skill<'a>(
    catalog: &'a SkillsCatalog,
    name: &str,
) -> Result<&'a SkillDefinition, ToolError> {
    catalog
        .skills
        .iter()
        .find(|skill| skill.name == name)
        .ok_or_else(|| invalid(format!("skill `{name}` not found")))
}

fn save_catalog(settings_dir: &Path, catalog: &SkillsCatalog) -> Result<(), ToolError> {
    if catalog.skills.is_empty() {
        return Err(invalid("skills catalog must not be empty".to_string()));
    }
    let rendered = catalog.render();
    SkillsCatalog::parse(&rendered).map_err(failed)?;
    crate::settings::write_skills_source(settings_dir, &rendered).map_err(failed)
}

fn skill_view(skill: &SkillDefinition, profile: &Profile) -> Value {
    let package = profile.settings.skill_packages.get(&skill.name);
    json!({
        "name": skill.name,
        "policy": skill.policy,
        "allowed_tools": skill.allowed_tools,
        "has_body": package.is_some(),
        "file_count": package.map(|item| item.files.len()).unwrap_or(0),
    })
}

fn validate_skill_fields(name: &str, policy: &str) -> Result<(), ToolError> {
    if name.contains('"') {
        return Err(invalid("skill name must not contain `\"`".to_string()));
    }
    if policy.contains('"') {
        return Err(invalid("skill policy must not contain `\"`".to_string()));
    }
    if name.trim().is_empty() {
        return Err(invalid("skill name must not be empty".to_string()));
    }
    Ok(())
}

fn normalize_tools(tools: &[String]) -> Result<Vec<String>, ToolError> {
    if tools.is_empty() {
        return Err(invalid(
            "at least one qualified tool (`server.tool`) is required".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(tools.len());
    for raw in tools {
        let qualified = QualifiedTool::parse(raw).map_err(invalid)?.qualified();
        if !out.iter().any(|tool| tool == &qualified) {
            out.push(qualified);
        }
    }
    Ok(out)
}

fn known_builtin_name(raw: &str) -> Result<String, ToolError> {
    let name = QualifiedTool::parse(raw).map_err(invalid)?.qualified();
    if !is_known_builtin(&name) {
        return Err(invalid(format!(
            "unknown built-in tool `{name}` in access.tools.builtins"
        )));
    }
    Ok(name)
}

fn provider_view(cfg: &AppConfig) -> Value {
    json!({
        "base_url": cfg.provider.base_url,
        "model": cfg.provider.model,
        "api_key_env": cfg.provider.api_key_env,
        "has_inline_api_key": cfg.provider.has_inline_api_key(),
        "timeout_ms": cfg.provider.timeout_ms,
        "max_retries": cfg.provider.max_retries,
        "retry_base_delay_ms": cfg.provider.retry_base_delay_ms,
        "effort": cfg.provider.effort,
        "thinking": cfg.provider.thinking,
    })
}

fn limits_view(cfg: &AppConfig) -> Value {
    json!({
        "max_iterations": cfg.limits.max_iterations,
        "max_tokens": cfg.limits.max_tokens,
        "max_duration_sec": cfg.limits.max_duration_sec,
        "max_cost": cfg.limits.max_cost.as_ref().map(|money| format!("{}:{}", money.currency(), money.amount())),
    })
}

fn mcp_public_view(server: &McpServerConfig) -> Value {
    match &server.transport {
        McpTransport::Stdio(stdio) => json!({
            "name": server.name,
            "transport": "stdio",
            "timeout_ms": server.timeout_ms,
            "command": stdio.command,
            "args": stdio.args,
            "cwd": stdio.cwd.as_ref().map(|path| path.display().to_string()),
        }),
        McpTransport::Http(http) => json!({
            "name": server.name,
            "transport": "http",
            "timeout_ms": server.timeout_ms,
            "url": http.url,
        }),
    }
}

fn find_mcp<'a>(cfg: &'a AppConfig, name: &str) -> Result<&'a McpServerConfig, ToolError> {
    cfg.mcp
        .iter()
        .find(|server| server.name == name)
        .ok_or_else(|| invalid(format!("MCP server `{name}` not found")))
}

fn build_mcp_add(args: &McpAddArgs) -> Result<McpServerConfig, ToolError> {
    let name = require_server_name(&args.name)?;
    let command = nonempty(args.command.as_deref());
    let url = nonempty(args.url.as_deref());
    let timeout_ms = args
        .timeout_ms
        .unwrap_or_else(McpServerConfig::default_timeout_ms);
    match (command, url) {
        (Some(_), Some(_)) | (None, None) => Err(invalid(
            "MCP server requires exactly one of command or url".to_string(),
        )),
        (Some(command), None) => Ok(McpServerConfig {
            name,
            timeout_ms,
            transport: McpTransport::Stdio(McpStdioConfig {
                command: command.to_string(),
                args: args.args.clone().unwrap_or_default(),
                env: HashMap::new(),
                cwd: optional_cwd(args.cwd.as_deref())?,
            }),
        }),
        (None, Some(url)) => {
            if args.args.as_ref().is_some_and(|args| !args.is_empty()) || args.cwd.is_some() {
                return Err(invalid(
                    "HTTP MCP servers must not set args or cwd".to_string(),
                ));
            }
            Ok(McpServerConfig {
                name,
                timeout_ms,
                transport: McpTransport::Http(McpHttpConfig {
                    url: url.to_string(),
                    headers: HashMap::new(),
                    auth: None,
                }),
            })
        }
    }
}

fn build_mcp_set(
    existing: &McpServerConfig,
    args: &McpSetArgs,
) -> Result<McpServerConfig, ToolError> {
    let command = nonempty(args.command.as_deref());
    let url = nonempty(args.url.as_deref());
    if command.is_some() && url.is_some() {
        return Err(invalid(
            "MCP server requires exactly one of command or url".to_string(),
        ));
    }
    let timeout_ms = args.timeout_ms.unwrap_or(existing.timeout_ms);
    if let Some(url) = url {
        if args.args.as_ref().is_some_and(|args| !args.is_empty()) || args.cwd.is_some() {
            return Err(invalid(
                "HTTP MCP servers must not set args or cwd".to_string(),
            ));
        }
        let headers = match &existing.transport {
            McpTransport::Http(http) => http.headers.clone(),
            McpTransport::Stdio(_) => HashMap::new(),
        };
        return Ok(McpServerConfig {
            name: existing.name.clone(),
            timeout_ms,
            transport: McpTransport::Http(McpHttpConfig {
                url: url.to_string(),
                headers,
                auth: match &existing.transport {
                    McpTransport::Http(http) => http.auth.clone(),
                    McpTransport::Stdio(_) => None,
                },
            }),
        });
    }
    if let Some(command) = command {
        let kept = stdio_parts(existing, args)?;
        return Ok(McpServerConfig {
            name: existing.name.clone(),
            timeout_ms,
            transport: McpTransport::Stdio(McpStdioConfig {
                command: command.to_string(),
                args: kept.args,
                env: kept.env,
                cwd: kept.cwd,
            }),
        });
    }
    match &existing.transport {
        McpTransport::Stdio(stdio) => {
            let kept = stdio_parts(existing, args)?;
            Ok(McpServerConfig {
                name: existing.name.clone(),
                timeout_ms,
                transport: McpTransport::Stdio(McpStdioConfig {
                    command: stdio.command.clone(),
                    args: kept.args,
                    env: kept.env,
                    cwd: kept.cwd,
                }),
            })
        }
        McpTransport::Http(http) => {
            if args.args.as_ref().is_some_and(|args| !args.is_empty()) || args.cwd.is_some() {
                return Err(invalid(
                    "HTTP MCP servers must not set args or cwd".to_string(),
                ));
            }
            Ok(McpServerConfig {
                name: existing.name.clone(),
                timeout_ms,
                transport: McpTransport::Http(http.clone()),
            })
        }
    }
}

struct KeptStdio {
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
}

fn stdio_parts(existing: &McpServerConfig, args: &McpSetArgs) -> Result<KeptStdio, ToolError> {
    let (prev_args, env, prev_cwd) = match &existing.transport {
        McpTransport::Stdio(stdio) => (stdio.args.clone(), stdio.env.clone(), stdio.cwd.clone()),
        McpTransport::Http(_) => (Vec::new(), HashMap::new(), None),
    };
    let cwd = match &args.cwd {
        Some(cwd) => optional_cwd(Some(cwd))?,
        None => prev_cwd,
    };
    Ok(KeptStdio {
        args: args.args.clone().unwrap_or(prev_args),
        env,
        cwd,
    })
}

fn require_server_name(name: &str) -> Result<String, ToolError> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(invalid(
            "MCP server name must be a single non-empty path segment".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn optional_cwd(cwd: Option<&str>) -> Result<Option<PathBuf>, ToolError> {
    match cwd.map(str::trim) {
        None => Ok(None),
        Some("") => Err(invalid("cwd must not be empty".to_string())),
        Some(cwd) => Ok(Some(PathBuf::from(cwd))),
    }
}

fn check_text(label: &str, content: &str) -> Result<(), ToolError> {
    if content.len() > MAX_PROFILE_TEXT_CHARS.saturating_mul(4)
        || content.chars().count() > MAX_PROFILE_TEXT_CHARS
    {
        return Err(invalid(format!(
            "{label} exceeds {MAX_PROFILE_TEXT_CHARS} characters"
        )));
    }
    Ok(())
}

fn bound_text(content: &str) -> (String, bool) {
    if content.chars().count() <= MAX_PROFILE_TEXT_CHARS {
        (content.to_string(), false)
    } else {
        (content.chars().take(MAX_PROFILE_TEXT_CHARS).collect(), true)
    }
}

fn text_view(field: &str, content: &str) -> Value {
    let (content, truncated) = bound_text(content);
    json!({ field: content, "truncated": truncated })
}

fn saved(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("applies".to_string(), json!("next_start"));
    }
    value
}

fn decode<T: for<'de> Deserialize<'de>>(tool: &str, value: Value) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|error| {
        ToolError::External(ExternalToolError::InvalidToolArguments {
            tool: format!("config.{tool}"),
            error: error.to_string(),
        })
    })
}

fn invalid(error: impl Into<String>) -> ToolError {
    ToolError::External(ExternalToolError::Failed {
        message: error.into(),
    })
}

fn failed(error: impl ToString) -> ToolError {
    ToolError::External(ExternalToolError::Failed {
        message: error.to_string(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentArgs {
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentArgs {
    #[serde(default)]
    agent: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NameArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NameContentArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillAddArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    policy: String,
    tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillSetArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    #[serde(default)]
    policy: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePutArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpAddArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpSetArgs {
    #[serde(default)]
    agent: Option<String>,
    name: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsArgs {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    max_iterations: Option<u32>,
    #[serde(default)]
    max_tokens: Option<u64>,
    #[serde(default)]
    max_duration_sec: Option<u64>,
    #[serde(default)]
    max_cost: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderArgs {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_retries: Option<u32>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    thinking: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolArgs {
    #[serde(default)]
    agent: Option<String>,
    tool: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InitToolArgs {
    agent: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{EffectiveToolPolicy, QualifiedTool, ResolvedAccessPolicy};
    use std::collections::BTreeSet;
    use std::fs;

    const SECRET: &str = "super-secret-key-marker";

    fn editor(root: &Path) -> ConfigEditor {
        ConfigEditor::new(Some(root.to_path_buf()), "demo")
    }

    fn scaffold(root: &Path, agent: &str) {
        init::run(&InitArgs {
            agent_id: agent.to_string(),
            project_root: root.to_path_buf(),
            force: false,
            interactive: false,
        })
        .expect("init");
    }

    #[test]
    fn rules_skill_and_mcp_stay_out_of_the_running_policy() {
        let dir = tempfile::tempdir().expect("tempdir");
        scaffold(dir.path(), "demo");
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([QualifiedTool::parse("home.read").unwrap()]);
        let policy = EffectiveToolPolicy::compile(&access, &skills, []);
        let before = policy.advertised();

        let editor = editor(dir.path());
        editor
            .call_sync(
                "rules_set",
                json!({"content": "Answer from the saved rules."}),
            )
            .expect("rules");
        editor
            .call_sync(
                "skill_add",
                json!({
                    "name": "notes",
                    "policy": "Keep notes short.",
                    "tools": ["home.read"]
                }),
            )
            .expect("skill");
        editor
            .call_sync(
                "skill_body_set",
                json!({"name": "notes", "content": "# Notes\n"}),
            )
            .expect("body");
        editor
            .call_sync(
                "skill_file_put",
                json!({"name": "notes", "path": "scripts/note.txt", "content": "hello\n"}),
            )
            .expect("file");
        editor
            .call_sync(
                "mcp_add",
                json!({"name": "docs", "command": "mcp-docs", "args": ["--stdio"]}),
            )
            .expect("mcp");

        assert_eq!(policy.advertised(), before);
        let rules = editor.call_sync("rules_get", json!({})).expect("get rules");
        assert_eq!(rules["rules"], "Answer from the saved rules.");
        let listed = editor.call_sync("mcp_list", json!({})).expect("list mcp");
        assert_eq!(listed["servers"][0]["command"], "mcp-docs");
        assert!(!listed.to_string().contains("env"));
    }

    #[test]
    fn mcp_cwd_env_and_headers_are_rejected_and_secrets_stay_hidden() {
        let dir = tempfile::tempdir().expect("tempdir");
        scaffold(dir.path(), "demo");
        let editor = editor(dir.path());

        let parent = editor
            .call_sync(
                "mcp_add",
                json!({"name": "local", "command": "mcp", "cwd": "../.kuibysheff/protected"}),
            )
            .expect_err("parent cwd");
        assert!(
            parent.to_string().contains("parent") || parent.to_string().contains("protected"),
            "{parent}"
        );
        let protected = editor
            .call_sync(
                "mcp_add",
                json!({"name": "local", "command": "mcp", "cwd": ".kuibysheff/protected/agents/x"}),
            )
            .expect_err("protected cwd");
        assert!(protected.to_string().contains("protected"), "{protected}");

        let env = editor
            .call_sync(
                "mcp_add",
                json!({"name": "local", "command": "mcp", "env": {"TOKEN": SECRET}}),
            )
            .expect_err("env");
        assert!(env.to_string().contains("env"), "{env}");
        let headers = editor
            .call_sync(
                "mcp_add",
                json!({"name": "remote", "url": "https://example.com/mcp", "headers": {"Authorization": SECRET}}),
            )
            .expect_err("headers");
        assert!(headers.to_string().contains("headers"), "{headers}");

        editor
            .call_sync("mcp_add", json!({"name": "local", "command": "mcp"}))
            .expect("add");
        let paths = resolve_agent_identity(dir.path(), "demo", None).expect("paths");
        let (mut cfg, _) = load_config(&paths.config).expect("load");
        if let McpTransport::Stdio(stdio) = &mut cfg.mcp[0].transport {
            stdio.env.insert("TOKEN".to_string(), SECRET.to_string());
        }
        let raw = serde_yaml::to_string(&cfg).expect("yaml");
        fs::write(&paths.config, raw).expect("write secret env");

        let view = editor
            .call_sync("mcp_get", json!({"name": "local"}))
            .expect("get");
        assert!(!view.to_string().contains(SECRET), "{view}");
        let show = editor.call_sync("show", json!({})).expect("show");
        assert!(!show.to_string().contains(SECRET), "{show}");
    }

    #[test]
    fn provider_url_and_key_of_the_running_agent_are_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        scaffold(dir.path(), "demo");
        scaffold(dir.path(), "other");
        let editor = editor(dir.path());

        let url = editor
            .call_sync("provider_set", json!({"base_url": "http://127.0.0.1:9/v1"}))
            .expect_err("self url");
        assert!(url.to_string().contains("base_url"), "{url}");
        let key = editor
            .call_sync("provider_set", json!({"api_key": SECRET}))
            .expect_err("api key");
        assert!(key.to_string().contains("api_key"), "{key}");
        let key_env = editor
            .call_sync("provider_set", json!({"api_key_env": "STOLEN_KEY"}))
            .expect_err("api key env");
        assert!(key_env.to_string().contains("api_key_env"), "{key_env}");

        editor
            .call_sync(
                "provider_set",
                json!({"agent": "other", "base_url": "http://127.0.0.1:11434/v1", "model": "local"}),
            )
            .expect("other url");
        let other = editor
            .call_sync("provider_get", json!({"agent": "other"}))
            .expect("other get");
        assert_eq!(other["base_url"], "http://127.0.0.1:11434/v1");
        assert_eq!(other["model"], "local");
        assert!(other.get("api_key").is_none());

        let paths = resolve_agent_identity(dir.path(), "demo", None).expect("paths");
        let raw = fs::read_to_string(&paths.config).expect("read");
        let raw = raw.replace(
            "api_key_env: \"OPENAI_API_KEY\"",
            "api_key_env: \"OPENAI_API_KEY\"\n  api_key: \"super-secret-key-marker\"",
        );
        fs::write(&paths.config, raw).expect("plant key");
        let shown = editor.call_sync("show", json!({})).expect("show");
        assert!(!shown.to_string().contains(SECRET), "{shown}");
        assert_eq!(shown["provider"]["has_inline_api_key"], true);
    }

    #[test]
    fn init_creates_another_profile_without_taking_a_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        scaffold(dir.path(), "demo");
        let editor = editor(dir.path());
        let created = editor
            .call_sync("init", json!({"agent": "fresh"}))
            .expect("init");
        assert_eq!(created["agent"], "fresh");
        assert_eq!(created["applies"], "next_start");
        let rejected = editor
            .call_sync("init", json!({"agent": "fresh", "api_key": SECRET}))
            .expect_err("key");
        assert!(rejected.to_string().contains("api_key"), "{rejected}");
    }
}
