//! Unified built-in tool descriptor registry.
//!
//! Advertising, access validation, and composite dispatch read from [`BUILTINS`].
//! Adding a built-in means one descriptor entry here plus the handler impl.

/// Which built-in executor handles a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinHandlerId {
    Home,
    LocalTools,
    Profile,
    Agent,
    Config,
}

/// How the tool is described in the runtime prompt fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinPrompt {
    /// Fixed JSON args example: `- {name} {json}`.
    StaticArgs(&'static str),
    /// `home.run` — example needs the configured program aliases.
    HomeRun,
    /// `home.read` — windowed home file read with next_offset pagination.
    HomeRead,
    /// `home.write` — file text in `arguments.content`.
    HomeWrite,
    /// `local_tools.read_file` — example mentions the workspace root.
    WorkspaceReadFile,
}

/// Single registration record for a built-in tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDescriptor {
    /// Qualified `server.tool` identity.
    pub name: &'static str,
    pub server: &'static str,
    pub tool: &'static str,
    pub description: &'static str,
    /// JSON Schema (draft-ish object) for tool arguments.
    pub schema_json: &'static str,
    /// Included when `access.mode: legacy`.
    pub legacy_default: bool,
    /// Prompt/runtime may require non-empty `access.run.programs`.
    pub requires_programs: bool,
    pub handler: BuiltinHandlerId,
    pub prompt: BuiltinPrompt,
}

impl ToolDescriptor {
    #[must_use]
    pub fn matches(&self, server: &str, tool: &str) -> bool {
        self.server == server && self.tool == tool
    }
}

/// How `home.write` is described to the model through Agent As MCP.
pub const HOME_WRITE_DESCRIPTION: &str = "Write a file under the agent home directory. Put the entire file text in arguments.content as one JSON string; escape quotes and newlines by JSON rules. File type does not select the field. Omit content and set body to \"raw\" only after a previous home.write of the same path was not executed because the JSON object was unterminated. Then mode is create, and after the JSON object write a newline, <<<FILE, a newline, the file bytes, a newline, and FILE>>>. Use mode append only after a previous home.write of the same path succeeded and the tool result includes bytes_written. A rejected turn writes nothing. One home.write per turn. Setting content and body \"raw\" together is rejected.";

/// Canonical built-in catalog. Policy, prompt, and availability derive from this slice.
pub const BUILTINS: &[ToolDescriptor] = &[
    ToolDescriptor {
        name: "home.list",
        server: "home",
        tool: "list",
        description: "List files under the agent home directory.",
        schema_json: r#"{"type":"object","properties":{"path":{"type":"string","default":"."}},"additionalProperties":false}"#,
        legacy_default: true,
        requires_programs: false,
        handler: BuiltinHandlerId::Home,
        prompt: BuiltinPrompt::StaticArgs(r#"{"path":"."}"#),
    },
    ToolDescriptor {
        name: "home.read",
        server: "home",
        tool: "read",
        description: "Read a UTF-8 character window from a file under the agent home directory. Use offset/next_offset to read large files in successive windows. There is no whole-file size reject.",
        schema_json: r#"{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"max_chars":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}"#,
        legacy_default: true,
        requires_programs: false,
        handler: BuiltinHandlerId::Home,
        prompt: BuiltinPrompt::HomeRead,
    },
    ToolDescriptor {
        name: "home.write",
        server: "home",
        tool: "write",
        description: HOME_WRITE_DESCRIPTION,
        schema_json: r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"},"body":{"type":"string","enum":["raw"]},"mode":{"type":"string","enum":["create","append"]}},"required":["path"],"additionalProperties":false}"#,
        legacy_default: true,
        requires_programs: false,
        handler: BuiltinHandlerId::Home,
        prompt: BuiltinPrompt::HomeWrite,
    },
    ToolDescriptor {
        name: "home.run",
        server: "home",
        tool: "run",
        description: "Run an allowlisted program in the sandboxed home environment.",
        schema_json: r#"{"type":"object","properties":{"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"timeout_ms":{"type":"integer","minimum":1}},"required":["program"],"additionalProperties":false}"#,
        legacy_default: false,
        requires_programs: true,
        handler: BuiltinHandlerId::Home,
        prompt: BuiltinPrompt::HomeRun,
    },
    ToolDescriptor {
        name: "local_tools.search_docs",
        server: "local_tools",
        tool: "search_docs",
        description: "Search documentation and source files in the workspace. No silent file-count/depth/size caps; only max_results limits returned hits.",
        schema_json: r#"{"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#,
        legacy_default: true,
        requires_programs: false,
        handler: BuiltinHandlerId::LocalTools,
        prompt: BuiltinPrompt::StaticArgs(r#"{"query":"phrase","max_results":8}"#),
    },
    ToolDescriptor {
        name: "local_tools.read_file",
        server: "local_tools",
        tool: "read_file",
        description: "Read a UTF-8 character window from a workspace file (not home). Use offset/next_offset for large files. There is no whole-file size reject.",
        schema_json: r#"{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"max_chars":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}"#,
        legacy_default: true,
        requires_programs: false,
        handler: BuiltinHandlerId::LocalTools,
        prompt: BuiltinPrompt::WorkspaceReadFile,
    },
    ToolDescriptor {
        name: "profile.list_skill",
        server: "profile",
        tool: "list_skill",
        description: "List files in one skill folder from the agent profile. Paths are relative to that folder.",
        schema_json: r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}"#,
        legacy_default: false,
        requires_programs: false,
        handler: BuiltinHandlerId::Profile,
        prompt: BuiltinPrompt::StaticArgs(r#"{"name":"research"}"#),
    },
    ToolDescriptor {
        name: "profile.read_skill",
        server: "profile",
        tool: "read_skill",
        description: "Read a UTF-8 character window from skill.md or a companion file in one skill folder. Omit path to read skill.md. Use offset/next_offset for large files.",
        schema_json: r#"{"type":"object","properties":{"name":{"type":"string"},"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"max_chars":{"type":"integer","minimum":1}},"required":["name"],"additionalProperties":false}"#,
        legacy_default: false,
        requires_programs: false,
        handler: BuiltinHandlerId::Profile,
        prompt: BuiltinPrompt::StaticArgs(
            r#"{"name":"research","path":"scripts/example.py","offset":0,"max_chars":50000}"#,
        ),
    },
    ToolDescriptor {
        name: "agent.list_tools",
        server: "agent",
        tool: "list_tools",
        description: "List tools this run may call. Each entry has server, name, and description. Denied tools are omitted.",
        schema_json: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        legacy_default: false,
        requires_programs: false,
        handler: BuiltinHandlerId::Agent,
        prompt: BuiltinPrompt::StaticArgs("{}"),
    },
    ToolDescriptor {
        name: "agent.describe_tool",
        server: "agent",
        tool: "describe_tool",
        description: "Return one allowed tool's description and inputSchema. A denied tool is rejected and its schema is not returned.",
        schema_json: r#"{"type":"object","properties":{"server":{"type":"string"},"tool":{"type":"string"}},"required":["server","tool"],"additionalProperties":false}"#,
        legacy_default: false,
        requires_programs: false,
        handler: BuiltinHandlerId::Agent,
        prompt: BuiltinPrompt::StaticArgs(r#"{"server":"home","tool":"write"}"#),
    },
    ToolDescriptor {
        name: "agent.guide",
        server: "agent",
        tool: "guide",
        description: "Static guide for configuring a saved agent profile. Contains no live profile data, secrets, or MCP commands.",
        schema_json: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        legacy_default: false,
        requires_programs: false,
        handler: BuiltinHandlerId::Agent,
        prompt: BuiltinPrompt::StaticArgs("{}"),
    },
    config_tool(
        "config.show",
        "show",
        "Show a redacted summary of a saved agent profile in this project. Secrets are omitted. Pass agent to read another id; omit it for the running agent.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.rules_get",
        "rules_get",
        "Read rules.md from a saved profile. The running process does not reload it.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.rules_set",
        "rules_set",
        "Replace rules.md in a saved profile. Applies on the next start.",
        CONTENT_SCHEMA,
    ),
    config_tool(
        "config.prompt_get",
        "prompt_get",
        "Read master_prompt.md from a saved profile.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.prompt_set",
        "prompt_set",
        "Replace master_prompt.md in a saved profile. Applies on the next start.",
        CONTENT_SCHEMA,
    ),
    config_tool(
        "config.skill_list",
        "skill_list",
        "List skills in a saved profile: name, policy, allowed_tools, and whether skill.md exists.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.skill_get",
        "skill_get",
        "Read one skill policy, allowed_tools, and skill.md from a saved profile.",
        NAME_SCHEMA,
    ),
    config_tool(
        "config.skill_add",
        "skill_add",
        "Add a skill to skills.dsl. tools are qualified server.tool names. policy must not contain a double quote.",
        SKILL_ADD_SCHEMA,
    ),
    config_tool(
        "config.skill_set",
        "skill_set",
        "Update a skill policy and/or allowed_tools. At least one of policy or tools is required.",
        SKILL_SET_SCHEMA,
    ),
    config_tool(
        "config.skill_remove",
        "skill_remove",
        "Remove a skill from skills.dsl and delete its skill folder. The last skill cannot be removed.",
        NAME_SCHEMA,
    ),
    config_tool(
        "config.skill_body_get",
        "skill_body_get",
        "Read skill.md for one skill.",
        NAME_SCHEMA,
    ),
    config_tool(
        "config.skill_body_set",
        "skill_body_set",
        "Write skill.md for a skill that already exists in skills.dsl. Applies on the next start.",
        NAME_CONTENT_SCHEMA,
    ),
    config_tool(
        "config.skill_file_list",
        "skill_file_list",
        "List files in one skill folder, including skill.md.",
        NAME_SCHEMA,
    ),
    config_tool(
        "config.skill_file_get",
        "skill_file_get",
        "Read a companion file inside a skill folder. skill.md is read with skill_body_get.",
        FILE_SCHEMA,
    ),
    config_tool(
        "config.skill_file_put",
        "skill_file_put",
        "Write a companion file inside a skill folder. skill.md must already exist. The path must stay inside the folder.",
        FILE_PUT_SCHEMA,
    ),
    config_tool(
        "config.skill_file_remove",
        "skill_file_remove",
        "Remove a companion file from a skill folder. Does not remove skill.md.",
        FILE_SCHEMA,
    ),
    config_tool(
        "config.mcp_list",
        "mcp_list",
        "List saved MCP servers without env, headers, or auth.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.mcp_get",
        "mcp_get",
        "Read one saved MCP server without env, headers, or auth. Does not connect.",
        NAME_SCHEMA,
    ),
    config_tool(
        "config.mcp_add",
        "mcp_add",
        "Add a saved MCP server with command (stdio) or url (http), plus args, cwd, and timeout. env and headers are rejected. The server is not started in this run. cwd must not contain .. or point at the protected store.",
        MCP_ADD_SCHEMA,
    ),
    config_tool(
        "config.mcp_set",
        "mcp_set",
        "Update a saved MCP server. Omitted fields stay. env and headers cannot be set or read. The server is not restarted in this run.",
        MCP_SET_SCHEMA,
    ),
    config_tool(
        "config.mcp_remove",
        "mcp_remove",
        "Remove a saved MCP server. The running process keeps servers it already connected.",
        NAME_SCHEMA,
    ),
    config_tool(
        "config.limits_get",
        "limits_get",
        "Read saved run limits.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.limits_set",
        "limits_set",
        "Update saved run limits. At least one field is required. Applies on the next start.",
        LIMITS_SCHEMA,
    ),
    config_tool(
        "config.provider_get",
        "provider_get",
        "Read saved provider settings. api_key is never returned.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.provider_set",
        "provider_set",
        "Update saved provider model, timeout_ms, max_retries, effort, or thinking. base_url is accepted only for a different agent id. api_key and api_key_env are rejected.",
        PROVIDER_SCHEMA,
    ),
    config_tool(
        "config.access_get",
        "access_get",
        "Read the saved access section. This does not change the running process.",
        AGENT_SCHEMA,
    ),
    config_tool(
        "config.builtins_add",
        "builtins_add",
        "Add one known built-in name to access.tools.builtins. Unknown names and filesystem or program changes are rejected. Applies on the next start.",
        TOOL_SCHEMA,
    ),
    config_tool(
        "config.builtins_remove",
        "builtins_remove",
        "Remove one built-in name from access.tools.builtins. Applies on the next start.",
        TOOL_SCHEMA,
    ),
    config_tool(
        "config.init",
        "init",
        "Create a new empty agent profile in this project from the template. Refuses an existing profile. Does not accept an API key.",
        INIT_SCHEMA,
    ),
];

const AGENT_SCHEMA: &str =
    r#"{"type":"object","properties":{"agent":{"type":"string"}},"additionalProperties":false}"#;
const CONTENT_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"content":{"type":"string"}},"required":["content"],"additionalProperties":false}"#;
const NAME_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"}},"required":["name"],"additionalProperties":false}"#;
const NAME_CONTENT_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"content":{"type":"string"}},"required":["name","content"],"additionalProperties":false}"#;
const SKILL_ADD_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"policy":{"type":"string"},"tools":{"type":"array","items":{"type":"string"},"minItems":1}},"required":["name","policy","tools"],"additionalProperties":false}"#;
const SKILL_SET_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"policy":{"type":"string"},"tools":{"type":"array","items":{"type":"string"}}},"required":["name"],"additionalProperties":false}"#;
const FILE_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"path":{"type":"string"}},"required":["name","path"],"additionalProperties":false}"#;
const FILE_PUT_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"path":{"type":"string"},"content":{"type":"string"}},"required":["name","path","content"],"additionalProperties":false}"#;
const MCP_ADD_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"command":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"url":{"type":"string"},"cwd":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1}},"required":["name"],"additionalProperties":false}"#;
const MCP_SET_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"name":{"type":"string"},"command":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"url":{"type":"string"},"cwd":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1}},"required":["name"],"additionalProperties":false}"#;
const LIMITS_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"max_iterations":{"type":"integer","minimum":1},"max_tokens":{"type":"integer","minimum":1},"max_duration_sec":{"type":"integer","minimum":1},"max_cost":{"type":"string"}},"additionalProperties":false}"#;
const PROVIDER_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"base_url":{"type":"string"},"model":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1},"max_retries":{"type":"integer","minimum":0},"effort":{"type":"string"},"thinking":{"type":"boolean"}},"additionalProperties":false}"#;
const TOOL_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"},"tool":{"type":"string"}},"required":["tool"],"additionalProperties":false}"#;
const INIT_SCHEMA: &str = r#"{"type":"object","properties":{"agent":{"type":"string"}},"required":["agent"],"additionalProperties":false}"#;

const fn config_tool(
    name: &'static str,
    tool: &'static str,
    description: &'static str,
    schema_json: &'static str,
) -> ToolDescriptor {
    ToolDescriptor {
        name,
        server: "config",
        tool,
        description,
        schema_json,
        legacy_default: false,
        requires_programs: false,
        handler: BuiltinHandlerId::Config,
        prompt: BuiltinPrompt::StaticArgs("{}"),
    }
}

/// Qualified names of every built-in tool.
pub fn known_builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|d| d.name)
}

/// Qualified names included in legacy mode (`access.mode: legacy`).
pub fn legacy_builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().filter(|d| d.legacy_default).map(|d| d.name)
}

/// Qualified `config.*` names. Not granted unless the profile allowlists them.
pub fn config_tool_names() -> impl Iterator<Item = &'static str> {
    BUILTINS
        .iter()
        .filter(|d| d.server == "config")
        .map(|d| d.name)
}

#[must_use]
pub fn is_known_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|d| d.name == name)
}

#[must_use]
pub fn is_builtin_server(server: &str) -> bool {
    BUILTINS.iter().any(|d| d.server == server)
}

#[must_use]
pub fn lookup(server: &str, tool: &str) -> Option<&'static ToolDescriptor> {
    BUILTINS.iter().find(|d| d.matches(server, tool))
}

#[must_use]
pub fn lookup_qualified(name: &str) -> Option<&'static ToolDescriptor> {
    BUILTINS.iter().find(|d| d.name == name)
}

/// Handler for a built-in server namespace (used for dispatch before MCP fallthrough).
#[must_use]
pub fn handler_for_server(server: &str) -> Option<BuiltinHandlerId> {
    BUILTINS
        .iter()
        .find(|d| d.server == server)
        .map(|d| d.handler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn builtins_are_unique_and_internally_consistent() {
        let mut names = HashSet::new();
        let mut pairs = HashSet::new();
        for d in BUILTINS {
            assert!(
                names.insert(d.name),
                "duplicate qualified name `{}`",
                d.name
            );
            assert_eq!(
                d.name,
                format!("{}.{}", d.server, d.tool),
                "name must equal server.tool"
            );
            assert!(
                pairs.insert((d.server, d.tool)),
                "duplicate server/tool pair"
            );
            assert!(
                serde_json::from_str::<serde_json::Value>(d.schema_json).is_ok(),
                "schema_json for `{}` must be valid JSON",
                d.name
            );
        }
    }

    #[test]
    fn legacy_is_subset_of_known_and_excludes_home_run() {
        let known: HashSet<_> = known_builtin_names().collect();
        let legacy: HashSet<_> = legacy_builtin_names().collect();
        assert!(legacy.is_subset(&known));
        let mut diff: Vec<_> = known.difference(&legacy).copied().collect();
        diff.sort_unstable();
        assert_eq!(
            diff,
            vec![
                "agent.describe_tool",
                "agent.guide",
                "agent.list_tools",
                "config.access_get",
                "config.builtins_add",
                "config.builtins_remove",
                "config.init",
                "config.limits_get",
                "config.limits_set",
                "config.mcp_add",
                "config.mcp_get",
                "config.mcp_list",
                "config.mcp_remove",
                "config.mcp_set",
                "config.prompt_get",
                "config.prompt_set",
                "config.provider_get",
                "config.provider_set",
                "config.rules_get",
                "config.rules_set",
                "config.show",
                "config.skill_add",
                "config.skill_body_get",
                "config.skill_body_set",
                "config.skill_file_get",
                "config.skill_file_list",
                "config.skill_file_put",
                "config.skill_file_remove",
                "config.skill_get",
                "config.skill_list",
                "config.skill_remove",
                "config.skill_set",
                "home.run",
                "profile.list_skill",
                "profile.read_skill",
            ]
        );
    }

    #[test]
    fn reserved_servers_match_descriptor_servers() {
        assert!(is_builtin_server("home"));
        assert!(is_builtin_server("local_tools"));
        assert!(is_builtin_server("profile"));
        assert!(is_builtin_server("agent"));
        assert!(is_builtin_server("config"));
        assert!(!is_builtin_server("docs"));
    }

    #[test]
    fn registry_names_cover_policy_known_set() {
        let names: Vec<_> = known_builtin_names().collect();
        assert_eq!(
            names,
            vec![
                "home.list",
                "home.read",
                "home.write",
                "home.run",
                "local_tools.search_docs",
                "local_tools.read_file",
                "profile.list_skill",
                "profile.read_skill",
                "agent.list_tools",
                "agent.describe_tool",
                "agent.guide",
                "config.show",
                "config.rules_get",
                "config.rules_set",
                "config.prompt_get",
                "config.prompt_set",
                "config.skill_list",
                "config.skill_get",
                "config.skill_add",
                "config.skill_set",
                "config.skill_remove",
                "config.skill_body_get",
                "config.skill_body_set",
                "config.skill_file_list",
                "config.skill_file_get",
                "config.skill_file_put",
                "config.skill_file_remove",
                "config.mcp_list",
                "config.mcp_get",
                "config.mcp_add",
                "config.mcp_set",
                "config.mcp_remove",
                "config.limits_get",
                "config.limits_set",
                "config.provider_get",
                "config.provider_set",
                "config.access_get",
                "config.builtins_add",
                "config.builtins_remove",
                "config.init",
            ]
        );
        for name in &names {
            assert!(is_known_builtin(name));
            assert!(lookup_qualified(name).is_some());
        }
    }
}
