use std::collections::{BTreeSet, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use thiserror::Error;

use crate::access::QualifiedTool;

static BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)skill\s+"([^"]+)"\s*\{(.*?)\}"#).expect("valid block regex")
});
static POLICY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"policy\s*:\s*"([^"]+)""#).expect("valid policy regex"));
static TOOLS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"allowed_tools\s*:\s*\[([^\]]*)\]").expect("valid tools regex"));
static QUOTED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]+)""#).expect("valid quoted regex"));

#[derive(Debug, Error)]
pub enum SkillsError {
    #[error("skills DSL parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillDefinition {
    pub name: String,
    pub policy: String,
    /// Qualified `server.tool` names only.
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SkillsCatalog {
    pub skills: Vec<SkillDefinition>,
}

impl SkillsCatalog {
    /// Parses the skills DSL into a catalog of skill definitions.
    ///
    /// Hard enforcement uses only qualified `allowed_tools` names. Skill `policy` text remains
    /// prompt guidance.
    ///
    /// # Errors
    ///
    /// Returns [`SkillsError`] if the DSL syntax is invalid or a tool name is not qualified.
    pub fn parse(source: &str) -> Result<Self, SkillsError> {
        let mut skills = Vec::new();
        for captures in BLOCK_RE.captures_iter(source) {
            let name = captures
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .ok_or_else(|| SkillsError::Parse("missing skill name".to_string()))?;
            let body = captures
                .get(2)
                .map(|m| m.as_str())
                .ok_or_else(|| SkillsError::Parse("missing skill body".to_string()))?;

            let policy = POLICY_RE
                .captures(body)
                .and_then(|x| x.get(1))
                .map(|m| m.as_str().trim().to_string())
                .ok_or_else(|| SkillsError::Parse(format!("skill `{name}` missing policy")))?;

            let tools_block = TOOLS_RE
                .captures(body)
                .and_then(|x| x.get(1))
                .map(|m| m.as_str())
                .ok_or_else(|| {
                    SkillsError::Parse(format!("skill `{name}` missing allowed_tools"))
                })?;

            let mut allowed_tools = Vec::new();
            for tool_capture in QUOTED_RE.captures_iter(tools_block) {
                let raw = tool_capture
                    .get(1)
                    .map(|inner| inner.as_str())
                    .ok_or_else(|| {
                        SkillsError::Parse(format!("skill `{name}` has an invalid tool entry"))
                    })?;
                let tool = QualifiedTool::parse(raw).map_err(|reason| {
                    SkillsError::Parse(format!("skill `{name}` allowed_tools: {reason}"))
                })?;
                allowed_tools.push(tool.qualified());
            }

            if allowed_tools.is_empty() {
                return Err(SkillsError::Parse(format!(
                    "skill `{name}` has empty allowed_tools"
                )));
            }
            skills.push(SkillDefinition {
                name,
                policy,
                allowed_tools,
            });
        }

        if skills.is_empty() {
            return Err(SkillsError::Parse("no skill blocks were found".to_string()));
        }

        Ok(Self { skills })
    }

    #[must_use]
    pub fn build_prompt_fragment(&self) -> String {
        let mut lines = vec![
            "Skills available to the agent:".to_string(),
            "Follow skill policies strictly when deciding tool usage.".to_string(),
            "Hard tool enforcement uses qualified names only; both built-in and MCP tools must be listed in a skill allowed_tools block.".to_string(),
        ];

        for skill in &self.skills {
            lines.push(format!(
                "- skill `{}` policy=`{}` allowed_tools={}",
                skill.name,
                skill.policy,
                skill.allowed_tools.join(",")
            ));
        }
        lines.join("\n")
    }

    /// Union of all skill `allowed_tools` as qualified tool identities.
    #[must_use]
    pub fn allowed_qualified_tools(&self) -> BTreeSet<QualifiedTool> {
        self.skills
            .iter()
            .flat_map(|skill| skill.allowed_tools.iter())
            .filter_map(|name| QualifiedTool::parse(name).ok())
            .collect()
    }

    #[must_use]
    pub fn allowed_tool_set(&self) -> HashSet<String> {
        self.skills
            .iter()
            .flat_map(|skill| skill.allowed_tools.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_dsl() {
        let parsed = SkillsCatalog::parse(
            r#"
            skill "research" {
              policy: "use_mcp_tools_first"
              allowed_tools: ["local_tools.search_docs", "local_tools.read_file"]
            }
            "#,
        )
        .expect("skills should parse");

        assert_eq!(parsed.skills.len(), 1);
        assert!(parsed
            .allowed_tool_set()
            .contains("local_tools.search_docs"));
        assert!(parsed
            .allowed_qualified_tools()
            .contains(&QualifiedTool::parse("local_tools.read_file").unwrap()));
    }

    #[test]
    fn rejects_bare_tool_names() {
        let err = SkillsCatalog::parse(
            r#"
            skill "research" {
              policy: "safe"
              allowed_tools: ["search_docs"]
            }
            "#,
        )
        .expect_err("bare names");
        assert!(err.to_string().contains("qualified"));
    }
}
