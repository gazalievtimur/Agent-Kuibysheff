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
    let mut home_run_allowed = false;
    for descriptor in BUILTINS {
        if !effective.allows_server_tool(descriptor.server, descriptor.tool) {
            continue;
        }
        if descriptor.requires_programs && access.programs().is_empty() {
            continue;
        }
        if let Some(line) = prompt_line_for(descriptor, access, workspace) {
            if descriptor.server == "home" && descriptor.tool == "run" {
                home_run_allowed = true;
            }
            builtins.push(line);
        }
    }

    if !builtins.is_empty() {
        lines.push("Available built-in tools:".to_string());
        lines.extend(builtins);
    }

    if home_run_allowed {
        append_home_run_sandbox_contract(&mut lines, access);
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
        lines.push("- Put short files in home.write arguments.content. Put BSL, XML, and any text with quotes or longer than a few hundred characters in a raw <<<FILE body after the JSON object (arguments.body = \"raw\"). One home.write per turn.".to_string());
    }

    lines.push("- When the goal is achieved, return done=true and fill `result`.".to_string());
    lines.push("- Return strict JSON and never use markdown.".to_string());

    lines.join("\n")
}

/// Injects the shared `home.run` sandbox contract for every entry point.
fn append_home_run_sandbox_contract(lines: &mut Vec<String>, access: &ResolvedAccessPolicy) {
    lines.push("home.run sandbox contract:".to_string());
    lines.push(
        "- home.run has no network; only the listed program aliases may be used.".to_string(),
    );
    for (alias, program) in access.programs() {
        let executable = program.executable.as_path().display();
        if program.runtime_read_roots.is_empty() {
            lines.push(format!("- program `{alias}` executable `{executable}`"));
        } else {
            let roots: Vec<String> = program
                .runtime_read_roots
                .iter()
                .map(|root| root.as_path().display().to_string())
                .collect();
            lines.push(format!(
                "- program `{alias}` executable `{executable}`; runtime_read_roots: {}",
                roots.join(", ")
            ));
        }
    }
    lines.push(
        "- Filesystem access is limited to home grants plus that program's runtime_read_roots; paths outside those roots are unreachable."
            .to_string(),
    );
    lines.push(
        "- On tool error containing `sandbox failure` or `sandbox unavailable`: do not retry the same call; prefer an MCP tool for the same job if available, else stop and record."
            .to_string(),
    );
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
        BuiltinPrompt::HomeWrite => Some(
            "- home.write {\"path\":\"out/file.md\",\"content\":\"short text\"} for a short file. For BSL, XML, or any text with quotes or longer than a few hundred characters, omit content and use {\"path\":\"out/src/Module.bsl\",\"body\":\"raw\",\"mode\":\"create\"}. After the JSON object write <<<FILE, a newline, the file bytes, a newline, and FILE>>>. Use mode append when a previous raw body was cut off."
                .to_string(),
        ),
        BuiltinPrompt::HomeRead => Some(format!(
            "- {} {{\"path\":\"relative/path\",\"offset\":0,\"max_chars\":50000}}. Reads a character window from the agent home directory. If truncated, continue with next_offset.",
            descriptor.name
        )),
        BuiltinPrompt::WorkspaceReadFile => Some(format!(
            "- {} {{\"path\":\"relative/path\",\"offset\":0,\"max_chars\":6000}}. Reads a character window from the workspace root (`{workspace}`), not from home. If truncated, continue with next_offset.",
            descriptor.name,
            workspace = workspace.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::access::config::{
        AccessPolicyConfig, FilesystemPolicyConfig, HomeFsPolicyConfig, ProgramPolicyConfig,
        RunPolicyConfig, ToolsPolicyConfig,
    };
    use crate::access::{AccessResolveInput, QualifiedTool, ResolvedAccessPolicy};
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

    fn home_run_access_and_policy() -> (ResolvedAccessPolicy, EffectiveToolPolicy) {
        let dir = tempdir().expect("tempdir");
        let runtime = dir.path().join("runtime");
        fs::create_dir_all(&runtime).expect("runtime");
        let exe_path = dir.path().join("python-stub");
        {
            let mut file = fs::File::create(&exe_path).expect("create exe");
            writeln!(file, "stub").expect("write");
        }

        let access_cfg = AccessPolicyConfig {
            tools: ToolsPolicyConfig {
                builtins: vec![
                    "home.list".to_string(),
                    "home.read".to_string(),
                    "home.write".to_string(),
                    "home.run".to_string(),
                ],
            },
            filesystem: FilesystemPolicyConfig {
                home: HomeFsPolicyConfig {
                    read: vec!["in".to_string(), "out".to_string()],
                    write: vec!["out".to_string()],
                },
                workspace: None,
                input_roots: Vec::new(),
            },
            run: RunPolicyConfig {
                programs: vec![ProgramPolicyConfig {
                    name: "python".to_string(),
                    executable: PathBuf::from("python-stub"),
                    runtime_read_roots: vec![PathBuf::from("runtime")],
                    inherit_env: Vec::new(),
                    allow_children: false,
                }],
                max_args: 32,
                max_arg_chars: 4096,
                max_output_chars: 200_000,
                max_timeout_ms: 120_000,
            },
            ..AccessPolicyConfig::default()
        };

        let access = ResolvedAccessPolicy::try_from(AccessResolveInput {
            access: Some(&access_cfg),
            config_dir: dir.path(),
        })
        .expect("resolve");
        // Keep temp paths alive while CanonicalRoot strings are displayed.
        let _kept = dir.keep();

        let skills = BTreeSet::from([
            QualifiedTool::parse("home.list").unwrap(),
            QualifiedTool::parse("home.read").unwrap(),
            QualifiedTool::parse("home.write").unwrap(),
            QualifiedTool::parse("home.run").unwrap(),
        ]);
        let effective = EffectiveToolPolicy::compile(&access, &skills, []);
        (access, effective)
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
        assert!(
            rules.contains("If truncated, continue with next_offset."),
            "home.read must teach window pagination: {rules}"
        );
        assert!(!rules.contains("- home.write"));
        assert!(!rules.contains("- home.run"));
        assert!(!rules.contains("local_tools.search_docs"));
        assert!(!rules.contains("local_tools.read_file"));
        assert!(
            !rules.contains("home.run sandbox contract"),
            "sandbox contract only when home.run is advertised: {rules}"
        );
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
        assert!(
            rules.contains("<<<FILE"),
            "home.write must teach the raw file body: {rules}"
        );
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
    fn runtime_rules_include_home_run_sandbox_contract() {
        let (access, effective) = home_run_access_and_policy();
        let rules = build_runtime_rules(
            &effective,
            &access,
            PathBuf::from("/home").as_path(),
            PathBuf::from("/workspace").as_path(),
        );

        assert!(
            rules.contains("- home.run"),
            "home.run must be advertised: {rules}"
        );
        assert!(
            rules.contains("no network"),
            "sandbox contract must state no network: {rules}"
        );
        assert!(
            rules.contains("sandbox failure"),
            "sandbox contract must teach no-retry on sandbox failure: {rules}"
        );
        assert!(
            rules.contains("program `python`"),
            "sandbox contract must list program aliases: {rules}"
        );
        assert!(
            rules.contains("runtime_read_roots"),
            "sandbox contract must mention runtime_read_roots: {rules}"
        );
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
                BuiltinPrompt::HomeRead
                | BuiltinPrompt::HomeWrite
                | BuiltinPrompt::WorkspaceReadFile => {}
                BuiltinPrompt::HomeRun => {
                    panic!("home.run should set requires_programs");
                }
            }
        }
    }
}
