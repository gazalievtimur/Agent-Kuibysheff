//! Agent As MCP: the run's allowed tools, with descriptions and argument schemas.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::access::EffectiveToolPolicy;
use crate::tool_api::{ExternalToolError, PolicyError, ToolError};

use super::registry::{self, BUILTINS};

/// One MCP tool passed in from the startup `tools/list` snapshot.
pub(crate) struct ExternalCatalogTool {
    pub(crate) server: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

#[derive(Debug, Clone)]
struct CatalogTool {
    server: String,
    name: String,
    description: String,
    input_schema: Value,
}

/// Snapshot of tools the current run may call.
#[derive(Debug, Clone, Default)]
pub(crate) struct AgentCatalog {
    tools: Vec<CatalogTool>,
}

impl AgentCatalog {
    #[must_use]
    pub(crate) fn from_effective(
        effective: &EffectiveToolPolicy,
        external: impl IntoIterator<Item = ExternalCatalogTool>,
    ) -> Self {
        let mut tools = Vec::new();
        for descriptor in BUILTINS {
            if !effective.allows_server_tool(descriptor.server, descriptor.tool) {
                continue;
            }
            let input_schema = serde_json::from_str(descriptor.schema_json)
                .unwrap_or_else(|_| json!({ "type": "object" }));
            tools.push(CatalogTool {
                server: descriptor.server.to_string(),
                name: descriptor.tool.to_string(),
                description: descriptor.description.to_string(),
                input_schema,
            });
        }
        for tool in external {
            if registry::is_builtin_server(&tool.server) {
                continue;
            }
            if !effective.allows_server_tool(&tool.server, &tool.name) {
                continue;
            }
            tools.push(CatalogTool {
                server: tool.server,
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            });
        }
        tools.sort_by(|left, right| (&left.server, &left.name).cmp(&(&right.server, &right.name)));
        Self { tools }
    }

    pub(crate) fn call(&self, tool: &str, arguments: Value) -> Result<Value, ToolError> {
        match tool {
            "list_tools" => Ok(self.list_tools()),
            "describe_tool" => self.describe_tool(arguments),
            other => Err(ToolError::External(ExternalToolError::UnknownTool {
                server: "agent".to_string(),
                tool: other.to_string(),
            })),
        }
    }

    fn list_tools(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "server": tool.server,
                    "name": tool.name,
                    "description": tool.description,
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    fn describe_tool(&self, arguments: Value) -> Result<Value, ToolError> {
        let args: DescribeArgs = serde_json::from_value(arguments).map_err(|error| {
            ToolError::External(ExternalToolError::InvalidToolArguments {
                tool: "agent.describe_tool".to_string(),
                error: error.to_string(),
            })
        })?;
        if args.server.trim().is_empty() || args.tool.trim().is_empty() {
            return Err(ToolError::External(
                ExternalToolError::InvalidToolArguments {
                    tool: "agent.describe_tool".to_string(),
                    error: "`server` and `tool` must not be empty".to_string(),
                },
            ));
        }
        let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.server == args.server && tool.name == args.tool)
        else {
            return Err(ToolError::Policy(PolicyError::ToolDenied {
                tool: format!("{}.{}", args.server, args.tool),
            }));
        };
        Ok(json!({
            "description": tool.description,
            "inputSchema": tool.input_schema,
        }))
    }
}

#[derive(Deserialize)]
struct DescribeArgs {
    server: String,
    tool: String,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::access::{EffectiveToolPolicy, QualifiedTool, ResolvedAccessPolicy};
    use crate::tools::registry::HOME_WRITE_DESCRIPTION;

    fn catalog() -> AgentCatalog {
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([
            QualifiedTool::parse("home.read").unwrap(),
            QualifiedTool::parse("home.write").unwrap(),
            QualifiedTool::parse("docs.search").unwrap(),
        ]);
        let mcp = [QualifiedTool::parse("docs.search").unwrap()];
        let mut effective = EffectiveToolPolicy::compile(&access, &skills, mcp);
        effective.grant_agent_catalog();
        AgentCatalog::from_effective(
            &effective,
            [
                ExternalCatalogTool {
                    server: "docs".to_string(),
                    name: "search".to_string(),
                    description: "Search docs.".to_string(),
                    input_schema: json!({"type": "object"}),
                },
                ExternalCatalogTool {
                    server: "docs".to_string(),
                    name: "secret".to_string(),
                    description: "hidden".to_string(),
                    input_schema: json!({"type": "object", "title": "SECRET_SCHEMA_MARKER"}),
                },
            ],
        )
    }

    #[test]
    fn list_tools_returns_allowed_descriptions_only() {
        let listed = catalog().call("list_tools", json!({})).expect("list");
        let tools = listed["tools"].as_array().expect("tools");
        let write = tools
            .iter()
            .find(|tool| tool["server"] == "home" && tool["name"] == "write")
            .expect("home.write");
        assert_eq!(write["description"], HOME_WRITE_DESCRIPTION);
        let description = write["description"].as_str().unwrap();
        assert!(description.contains("arguments.content"));
        assert!(description.contains("unterminated"));
        assert!(!description.contains("short"));
        assert!(!description.contains("medium"));
        assert!(!description.contains("few hundred"));
        assert!(tools.iter().any(|tool| tool["name"] == "list_tools"));
        assert!(tools
            .iter()
            .any(|tool| tool["server"] == "docs" && tool["name"] == "search"));
        assert!(!tools.iter().any(|tool| tool["name"] == "secret"));
        assert!(!listed.to_string().contains("SECRET_SCHEMA_MARKER"));
    }

    #[test]
    fn describe_tool_returns_home_write_schema() {
        let described = catalog()
            .call("describe_tool", json!({"server": "home", "tool": "write"}))
            .expect("describe");
        let properties = &described["inputSchema"]["properties"];
        assert!(properties.get("content").is_some());
        assert!(properties.get("body").is_some());
        assert!(properties.get("mode").is_some());
        assert!(described["description"]
            .as_str()
            .unwrap()
            .contains("arguments.content"));
    }

    #[test]
    fn describe_tool_denies_a_tool_outside_the_catalog() {
        let error = catalog()
            .call("describe_tool", json!({"server": "docs", "tool": "secret"}))
            .expect_err("denied");
        let text = error.to_string();
        assert!(text.contains("docs.secret"));
        assert!(!text.contains("SECRET_SCHEMA_MARKER"));
        assert!(!text.contains("arguments.content"));
    }
}
