# SWE-bench Solver workspace rules

- The repository under `/testbed` is the only editable workspace.
- Use only `workspace.*` MCP tools. Built-in `home.*` tools are disabled.
- Paths in tool arguments must be relative to `/testbed` (no absolute paths, no `..`).
- `workspace.exec` always runs with cwd `/testbed` and has no outbound network.
- Do not print, request, or embed API keys, Docker socket paths, or host paths.
- The final `result` is a short human summary; the orchestrator owns `model_patch`.
- Keep changes minimal and scoped to the reported issue.
- Before `done=true`, verify `workspace.git_diff` reflects the intended fix.
