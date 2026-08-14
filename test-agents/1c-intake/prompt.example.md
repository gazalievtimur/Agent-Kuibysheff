# 1c-intake prompt examples

## Jira issue key

```text
Собери первичную информацию по задаче PROJ-123 (product=demo).

Goal: проверяемый brief из Jira/Confluence без анализа кода конфигурации.

Required steps:
1. Fetch the Jira issue and linked Confluence pages through atlassian MCP tools.
2. Describe screenshots if the model supports vision.
3. Write out/task_brief.md with tz_status.
4. Write out/sources.json and out/manifest.json (apply_mode=none).
5. Final response: done=true with a short result.

Return JSON only on every turn.
```

PowerShell:

```powershell
cargo run --bin kbshff -- init 1c-intake --project-root . --force
cargo run --bin kbshff -- config --project-root . --agent 1c-intake `
  import --from .\test-agents\1c-intake --force

cargo run --bin kbshff -- run `
  --project-root . `
  --agent 1c-intake `
  --home homes/manual-intake `
  --prompt "Собери первичную информацию по задаче PROJ-123"
```

Or via orchestrator:

```powershell
.\scripts\1c-dev-run.ps1 -Product demo -IssueKey PROJ-123 -Stage 1
```
