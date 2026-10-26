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
    /// `home.write` — short `content` or a raw body after the JSON object.
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
        description: "Write a file under the agent home directory. Short text goes in content. BSL, XML, and any text with quotes or longer than a few hundred characters uses body \"raw\" plus a <<<FILE block after the JSON object. mode is create (default) or append.",
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
];

/// Qualified names of every built-in tool.
pub fn known_builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|d| d.name)
}

/// Qualified names included in legacy mode (`access.mode: legacy`).
pub fn legacy_builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().filter(|d| d.legacy_default).map(|d| d.name)
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
            vec!["home.run", "profile.list_skill", "profile.read_skill"]
        );
    }

    #[test]
    fn reserved_servers_match_descriptor_servers() {
        assert!(is_builtin_server("home"));
        assert!(is_builtin_server("local_tools"));
        assert!(is_builtin_server("profile"));
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
            ]
        );
        for name in &names {
            assert!(is_known_builtin(name));
            assert!(lookup_qualified(name).is_some());
        }
    }
}
