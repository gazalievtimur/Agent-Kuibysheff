//! Static configuration guide returned by `agent.guide`.
//!
//! The text is fixed. It must not be filled with the live profile, secrets, or
//! MCP commands.

/// Operator manual for configuring a saved agent profile.
pub const AGENT_GUIDE: &str = "\
Agent profile layout:
- An agent id lives under .kuibysheff/protected/agents/<id>/ in the project root.
- agent-config.yaml holds provider, limits, mcp, and access.
- master_prompt.md, rules.md, and skills.dsl sit beside that file.
- Skill folders hold skill.md and companion files. .kuibysheff/runtime/skills/<id>/<name>/ wins over protected/agents/<id>/skills/<name>/ when both exist.

config.* tools, when list_tools includes them, write that saved profile:
- rules and the master prompt
- skills: policy, allowed_tools, skill.md, and companion files inside the skill folder
- MCP servers: command or url, args, cwd, and timeout. cwd must not contain .. and must not point at the protected store. env, headers, and auth are not accepted and are not returned. The server is not started in this run.
- limits
- provider model, timeout_ms, max_retries, effort, and thinking
- provider.base_url only when agent is a different id in this project. The running agent's base_url stays unchanged.
- access.tools.builtins, and only names of known built-in tools
- config.init creates another empty profile from the template and does not take an API key

Edits apply on the next start. This process does not reload access, MCP, or the prompt.

These tools do not read or write .env, api_key, or api_key_env. They do not replace the whole document, import a bundle, change filesystem grants, change program aliases, or change billing and event-mcp.

Do not launch this agent executable with home.run. That would bypass these tools.

If config.* is absent from list_tools, this profile does not grant them. Say so. Do not try to edit the protected store through other tools.
";
