use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
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
    #[error("skill package `{0}` is not declared in skills.dsl")]
    UndeclaredPackage(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub name: String,
    pub policy: String,
    /// Qualified `server.tool` names only.
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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

    /// Emits canonical skills DSL source.
    ///
    /// Skills are separated by a blank line. Tool lists use qualified `server.tool` names.
    #[must_use]
    pub fn render(&self) -> String {
        let mut blocks = Vec::with_capacity(self.skills.len());
        for skill in &self.skills {
            let tools = skill
                .allowed_tools
                .iter()
                .map(|tool| format!("\"{tool}\""))
                .collect::<Vec<_>>()
                .join(", ");
            blocks.push(format!(
                "skill \"{}\" {{\n  policy: \"{}\"\n  allowed_tools: [{tools}]\n}}",
                skill.name, skill.policy
            ));
        }
        blocks.join("\n\n")
    }

    /// Prompt catalog. `file_counts` maps a skill name to the number of files in its folder.
    ///
    /// A missing or zero count means the skill has no stored instructions.
    #[must_use]
    pub fn build_prompt_fragment(&self, file_counts: &BTreeMap<String, usize>) -> String {
        let mut lines = vec![
            "Skills available to the agent:".to_string(),
            "Follow skill policies strictly when deciding tool usage.".to_string(),
            "Hard tool enforcement uses qualified names only; both built-in and MCP tools must be listed in a skill allowed_tools block.".to_string(),
        ];
        let any_stored = self
            .skills
            .iter()
            .any(|skill| file_counts.get(&skill.name).copied().unwrap_or(0) > 0);
        if any_stored {
            lines.push(
                "When a skill has instructions=stored, call profile.read_skill {\"name\":\"<skill>\"} to read skill.md, then profile.list_skill {\"name\":\"<skill>\"} and profile.read_skill {\"name\":\"<skill>\",\"path\":\"<relative>\"} for companion files. The policy line is only a summary. Do not run scripts from the skill folder. Copy a script into the home directory with home.write and run it with home.run only when that tool is allowed.".to_string(),
            );
        }

        for skill in &self.skills {
            let files = file_counts.get(&skill.name).copied().unwrap_or(0);
            let instructions = if files > 0 { "stored" } else { "none" };
            lines.push(format!(
                "- skill `{}` policy=`{}` instructions={instructions} files={files} allowed_tools={}",
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
    #[cfg_attr(not(test), allow(dead_code))]
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

    #[test]
    fn render_round_trips_through_parse() {
        let src = r#"
            skill "research" {
              policy: "use_mcp_tools_first"
              allowed_tools: ["local_tools.search_docs", "local_tools.read_file"]
            }

            skill "edit" {
              policy: "prefer_small_diffs"
              allowed_tools: ["home.write_file"]
            }
            "#;
        let parsed = SkillsCatalog::parse(src).expect("initial parse");
        let rendered = parsed.render();
        let round_trip = SkillsCatalog::parse(&rendered).expect("parse rendered");
        assert_eq!(round_trip, parsed);
        assert!(rendered.contains("skill \"research\" {"));
        assert!(rendered
            .contains("allowed_tools: [\"local_tools.search_docs\", \"local_tools.read_file\"]"));
        assert!(rendered.contains("\n\nskill \"edit\""));
    }

    #[test]
    fn prompt_fragment_marks_stored_instructions_and_file_count() {
        let catalog = SkillsCatalog::parse(
            r#"
            skill "research" {
              policy: "read first"
              allowed_tools: ["home.read"]
            }
            skill "workspace" {
              policy: "write home"
              allowed_tools: ["home.write"]
            }
            "#,
        )
        .expect("parse");
        let counts = BTreeMap::from([("research".to_string(), 2usize)]);
        let fragment = catalog.build_prompt_fragment(&counts);
        assert!(fragment.contains("instructions=stored files=2"));
        assert!(fragment.contains("instructions=none files=0"));
        assert!(fragment.contains("profile.read_skill"));
        assert!(fragment.contains("profile.list_skill"));
        assert!(fragment.contains("Do not run scripts from the skill folder"));
    }
}
