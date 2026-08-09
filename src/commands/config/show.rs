//! `config show` — short profile summary.

use serde::Serialize;

use crate::cli::ConfigArgs;
use crate::skills::dsl::SkillsCatalog;

use super::common::load_profile;
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct ShowData {
    agent: String,
    provider_model: String,
    provider_base_url: String,
    mcp_servers: usize,
    skills: usize,
    limits: LimitsSummary,
    has_rules: bool,
    access_present: bool,
}

#[derive(Debug, Serialize)]
struct LimitsSummary {
    max_iterations: u32,
    max_tokens: u64,
    max_duration_sec: u64,
    max_cost: Option<String>,
}

pub fn run(args: &ConfigArgs) -> Result<(), ConfigCmdError> {
    let profile = load_profile(&args.identity)?;
    let skills = SkillsCatalog::parse(&profile.settings.skills_source)?;
    let max_cost = profile
        .config
        .limits
        .max_cost
        .as_ref()
        .map(|m| format!("{}:{}", m.currency(), m.amount()));
    let data = ShowData {
        agent: profile.paths.agent_id.clone(),
        provider_model: profile.config.provider.model.clone(),
        provider_base_url: profile.config.provider.base_url.clone(),
        mcp_servers: profile.config.mcp.len(),
        skills: skills.skills.len(),
        limits: LimitsSummary {
            max_iterations: profile.config.limits.max_iterations,
            max_tokens: profile.config.limits.max_tokens,
            max_duration_sec: profile.config.limits.max_duration_sec,
            max_cost,
        },
        has_rules: !profile.settings.rules.trim().is_empty(),
        access_present: profile.config.access.is_some(),
    };
    emit_ok(args.format, "profile", "show", &data)
}
