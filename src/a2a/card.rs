//! Build an A2A [`AgentCard`] from a Kuibyshev agent profile.

use std::collections::HashMap;

use a2a::{
    AgentCapabilities, AgentCard, AgentInterface, AgentSkill, HttpAuthSecurityScheme,
    SecurityScheme, TRANSPORT_PROTOCOL_HTTP_JSON, TRANSPORT_PROTOCOL_JSONRPC,
};
use a2a_server::StaticAgentCard;
use anyhow::{Context, Result};

use crate::settings::load_settings;
use crate::skills::dsl::SkillsCatalog;

const BEARER_SCHEME_NAME: &str = "bearer";

/// Shared A2A capability flags for the Agent Card and request handler.
#[must_use]
pub fn default_agent_capabilities() -> AgentCapabilities {
    AgentCapabilities {
        streaming: Some(true),
        push_notifications: Some(false),
        extensions: None,
        extended_agent_card: Some(false),
    }
}

/// Options for Agent Card construction.
#[derive(Debug, Clone)]
pub struct CardOptions {
    pub agent_id: String,
    pub settings_dir: std::path::PathBuf,
    pub public_url: String,
    pub require_bearer: bool,
}

/// Load profile settings/skills and build a static Agent Card producer.
///
/// # Errors
///
/// Returns an error when settings or skills cannot be loaded/parsed.
pub fn build_static_card(opts: &CardOptions) -> Result<StaticAgentCard> {
    let card = build_agent_card(opts)?;
    Ok(StaticAgentCard::new(card))
}

/// Build an [`AgentCard`] for the given profile and public base URL.
///
/// # Errors
///
/// Returns an error when settings or skills cannot be loaded/parsed.
pub fn build_agent_card(opts: &CardOptions) -> Result<AgentCard> {
    let settings = load_settings(&opts.settings_dir).with_context(|| {
        format!(
            "load settings for A2A card from {}",
            opts.settings_dir.display()
        )
    })?;
    let skills = SkillsCatalog::parse(&settings.skills_source)
        .with_context(|| "parse skills.dsl for A2A Agent Card")?;

    let base = opts.public_url.trim_end_matches('/');
    let supported_interfaces = vec![
        AgentInterface::new(format!("{base}/jsonrpc"), TRANSPORT_PROTOCOL_JSONRPC),
        AgentInterface::new(format!("{base}/rest"), TRANSPORT_PROTOCOL_HTTP_JSON),
    ];

    let description = truncate_description(&settings.master_prompt);
    let a2a_skills: Vec<AgentSkill> = skills
        .skills
        .iter()
        .map(|skill| AgentSkill {
            id: skill.name.clone(),
            name: skill.name.clone(),
            description: skill.policy.clone(),
            tags: vec!["kuibysheff".into()],
            examples: None,
            input_modes: None,
            output_modes: None,
            security_requirements: None,
        })
        .collect();

    let (security_schemes, security_requirements) = if opts.require_bearer {
        let mut schemes = HashMap::new();
        schemes.insert(
            BEARER_SCHEME_NAME.to_string(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".into(),
                description: Some("Bearer token from --token-env".into()),
                bearer_format: None,
            }),
        );
        let mut req = HashMap::new();
        req.insert(BEARER_SCHEME_NAME.to_string(), Vec::new());
        (Some(schemes), Some(vec![req]))
    } else {
        (None, None)
    };

    Ok(AgentCard {
        name: opts.agent_id.clone(),
        description,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_interfaces,
        capabilities: default_agent_capabilities(),
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: a2a_skills,
        provider: Some(a2a::AgentProvider {
            organization: "agent_Kuibysheff".into(),
            url: "https://github.com/gybson63/Agent-Kuibysheff".into(),
        }),
        documentation_url: None,
        icon_url: None,
        security_schemes,
        security_requirements,
        signatures: None,
    })
}

fn truncate_description(master_prompt: &str) -> String {
    let trimmed = master_prompt.trim();
    let first_para = trimmed
        .split("\n\n")
        .next()
        .unwrap_or(trimmed)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    const MAX: usize = 280;
    if first_para.chars().count() <= MAX {
        first_para
    } else {
        let truncated: String = first_para.chars().take(MAX.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Resolve `public_url` from CLI or default `http://{bind}`.
#[must_use]
pub fn resolve_public_url(bind: &str, public_url: Option<&str>) -> String {
    public_url
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("http://{bind}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn truncate_keeps_short_text() {
        assert_eq!(truncate_description("hello"), "hello");
    }

    #[test]
    fn builds_card_with_interfaces() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("master_prompt.md"),
            "You are a demo agent.\n\nMore details.",
        )
        .unwrap();
        fs::write(
            dir.path().join("skills.dsl"),
            r#"
skill "demo" {
  policy: "be helpful"
  allowed_tools: ["home.read"]
}
"#,
        )
        .unwrap();

        let card = build_agent_card(&CardOptions {
            agent_id: "demo".into(),
            settings_dir: dir.path().to_path_buf(),
            public_url: "http://127.0.0.1:8787".into(),
            require_bearer: true,
        })
        .expect("card");

        assert_eq!(card.name, "demo");
        assert!(card.description.contains("demo agent"));
        assert_eq!(card.supported_interfaces.len(), 2);
        assert_eq!(
            card.supported_interfaces[0].url,
            "http://127.0.0.1:8787/jsonrpc"
        );
        assert_eq!(card.skills.len(), 1);
        assert!(card.security_schemes.is_some());
        assert_eq!(card.capabilities.streaming, Some(true));
        assert_eq!(card.capabilities.push_notifications, Some(false));
    }
}
