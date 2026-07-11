use std::collections::HashSet;

use regex::Regex;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillsError {
    #[error("skills DSL parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillDefinition {
    pub name: String,
    pub policy: String,
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SkillsCatalog {
    pub skills: Vec<SkillDefinition>,
}

impl SkillsCatalog {
    /// Parses the skills DSL into a catalog of skill definitions.
    ///
    /// # Errors
    ///
    /// Returns [`SkillsError`] if the DSL syntax is invalid.
    pub fn parse(source: &str) -> Result<Self, SkillsError> {
        let block_re = Regex::new(r#"(?s)skill\s+"([^"]+)"\s*\{(.*?)\}"#)
            .map_err(|err| SkillsError::Parse(err.to_string()))?;
        let policy_re = Regex::new(r#"policy\s*:\s*"([^"]+)""#)
            .map_err(|err| SkillsError::Parse(err.to_string()))?;
        let tools_re = Regex::new(r"allowed_tools\s*:\s*\[([^\]]*)\]")
            .map_err(|err| SkillsError::Parse(err.to_string()))?;
        let quoted_re =
            Regex::new(r#""([^"]+)""#).map_err(|err| SkillsError::Parse(err.to_string()))?;

        let mut skills = Vec::new();
        for captures in block_re.captures_iter(source) {
            let name = captures
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .ok_or_else(|| SkillsError::Parse("missing skill name".to_string()))?;
            let body = captures
                .get(2)
                .map(|m| m.as_str())
                .ok_or_else(|| SkillsError::Parse("missing skill body".to_string()))?;

            let policy = policy_re
                .captures(body)
                .and_then(|x| x.get(1))
                .map(|m| m.as_str().trim().to_string())
                .ok_or_else(|| SkillsError::Parse(format!("skill `{name}` missing policy")))?;

            let tools_block = tools_re
                .captures(body)
                .and_then(|x| x.get(1))
                .map(|m| m.as_str())
                .ok_or_else(|| {
                    SkillsError::Parse(format!("skill `{name}` missing allowed_tools"))
                })?;

            let allowed_tools = quoted_re
                .captures_iter(tools_block)
                .filter_map(|m| m.get(1).map(|inner| inner.as_str().to_string()))
                .collect::<Vec<_>>();

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
              allowed_tools: ["search_docs", "read_file"]
            }
            "#,
        )
        .expect("skills should parse");

        assert_eq!(parsed.skills.len(), 1);
        assert!(parsed.allowed_tool_set().contains("search_docs"));
    }
}
