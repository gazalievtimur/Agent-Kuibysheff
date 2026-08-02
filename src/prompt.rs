use std::path::Path;

use crate::access::{EffectiveToolPolicy, ResolvedAccessPolicy};
use crate::tools::registry::{self, BuiltinPrompt, BUILTINS};

/// Builds the dynamic runtime rules section of the system prompt.
///
/// Only tools that are actually in the effective allowlist are described, so the
/// model is not encouraged to call tools that would be denied at runtime.
pub fn build_runtime_rules(
    effective: &EffectiveToolPolicy,
    access: &ResolvedAccessPolicy,
    home: &Path,
    workspace: &Path,
) -> String {
    let mut lines = vec![
        "Runtime rules:".to_string(),
        "- Stay within configured limits.".to_string(),
        format!(
            "- The home directory is `{home}`. All file writes must use home.write and paths relative to this directory.",
            home = home.display()
        ),
        "- Input files are read-only context and are not copied into home automatically."
            .to_string(),
    ];

    let mut builtins = Vec::new();
    for descriptor in BUILTINS {
        if !effective.allows_server_tool(descriptor.server, descriptor.tool) {
            continue;
        }
        if descriptor.requires_programs && access.programs().is_empty() {
            continue;
        }
        if let Some(line) = prompt_line_for(descriptor, access, workspace) {
            builtins.push(line);
        }
    }

    if !builtins.is_empty() {
        lines.push("Available built-in tools:".to_string());
        lines.extend(builtins);
    }

    let mcp_tools: Vec<String> = effective
        .tools()
        .iter()
        .filter(|t| !registry::is_builtin_server(t.server()))
        .map(|t| format!("- {}.{}", t.server(), t.tool()))
        .collect();

    if !mcp_tools.is_empty() {
        lines.push("Available MCP tools:".to_string());
        lines.extend(mcp_tools);
    }

    if effective.allows_server_tool("home", "write") {
        lines.push("- For coding tasks, write deliverables under out/ and create out/manifest.json according to the orchestrator contract in the supplied rules.".to_string());
    }

    lines.push("- When the goal is achieved, return done=true and fill `result`.".to_string());
    lines.push("- Return strict JSON and never use markdown.".to_string());

    lines.join("\n")
}

fn prompt_line_for(
    descriptor: &registry::ToolDescriptor,
    access: &ResolvedAccessPolicy,
    workspace: &Path,
) -> Option<String> {
    match descriptor.prompt {
        BuiltinPrompt::StaticArgs(args) => Some(format!("- {} {args}", descriptor.name)),
        BuiltinPrompt::HomeRun => {
            let aliases: Vec<&str> = access.programs().keys().map(|a| a.as_str()).collect();
            let example = aliases.first().copied().unwrap_or("python");
            let programs = aliases.join(", ");
            Some(format!(
                "- {} {{\"program\":\"{example}\",\"args\":[\"solution.py\"],\"timeout_ms\":30000}} (available programs: {programs})",
                descriptor.name
            ))
        }
        BuiltinPrompt::WorkspaceReadFile => Some(format!(
            "- {} {{\"path\":\"relative/path\",\"max_chars\":6000}}. Reads from the workspace root (`{workspace}`), not from home.",
            descriptor.name,
            workspace = workspace.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::*;
    use crate::access::QualifiedTool;
    use crate::tools::registry;

    fn home_read_policy() -> EffectiveToolPolicy {
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([QualifiedTool::parse("home.read").unwrap()]);
        EffectiveToolPolicy::compile(&access, &skills, [])
    }

    fn all_builtins_policy() -> EffectiveToolPolicy {
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([
            QualifiedTool::parse("home.list").unwrap(),
            QualifiedTool::parse("home.read").unwrap(),
            QualifiedTool::parse("home.write").unwrap(),
            QualifiedTool::parse("local_tools.search_docs").unwrap(),
            QualifiedTool::parse("local_tools.read_file").unwrap(),
        ]);
        EffectiveToolPolicy::compile(&access, &skills, [])
    }

    fn mcp_policy() -> EffectiveToolPolicy {
        let access = ResolvedAccessPolicy::legacy();
        let skills = BTreeSet::from([
            QualifiedTool::parse("home.read").unwrap(),
            QualifiedTool::parse("docs.search").unwrap(),
        ]);
        let mcp = [QualifiedTool::parse("docs.search").unwrap()];
        EffectiveToolPolicy::compile(&access, &skills, mcp)
    }

    #[test]
    fn runtime_rules_mention_only_allowed_tools() {
        let rules = build_runtime_rules(
            &home_read_policy(),
            &ResolvedAccessPolicy::legacy(),
            PathBuf::from("/home").as_path(),
            PathBuf::from("/workspace").as_path(),
        );

        assert!(rules.contains("- home.read"));
        assert!(!rules.contains("- home.write"));
        assert!(!rules.contains("- home.run"));
        assert!(!rules.contains("local_tools.search_docs"));
        assert!(!rules.contains("local_tools.read_file"));
    }

    #[test]
    fn runtime_rules_include_all_allowed_builtins() {
        let rules = build_runtime_rules(
            &all_builtins_policy(),
            &ResolvedAccessPolicy::legacy(),
            PathBuf::from("/home").as_path(),
            PathBuf::from("/workspace").as_path(),
        );

        assert!(rules.contains("home.list"));
        assert!(rules.contains("home.read"));
        assert!(rules.contains("home.write"));
        assert!(rules.contains("local_tools.search_docs"));
        assert!(rules.contains("local_tools.read_file"));
        assert!(!rules.contains("home.run"));
    }

    #[test]
    fn runtime_rules_include_mcp_tools() {
        let rules = build_runtime_rules(
            &mcp_policy(),
            &ResolvedAccessPolicy::legacy(),
            PathBuf::from("/home").as_path(),
            PathBuf::from("/workspace").as_path(),
        );

        assert!(rules.contains("Available MCP tools:"));
        assert!(rules.contains("- docs.search"));
        assert!(rules.contains("home.read"));
    }

    #[test]
    fn advertised_builtins_match_registry_prompt_coverage() {
        // Every non-requires_programs builtin must be representable in the prompt.
        for descriptor in registry::BUILTINS {
            if descriptor.requires_programs {
                continue;
            }
            match descriptor.prompt {
                BuiltinPrompt::StaticArgs(args) => {
                    assert!(args.starts_with('{'), "static args for {}", descriptor.name);
                }
                BuiltinPrompt::WorkspaceReadFile => {}
                BuiltinPrompt::HomeRun => {
                    panic!("home.run should set requires_programs");
                }
            }
        }
    }
}
