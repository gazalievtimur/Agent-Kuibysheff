# 1c-intake prompt examples

## Jira issue key

```text
Собери первичную информацию по задаче K7-20486 (product=k7).

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
cargo run --bin agent_Kuibyshev -- run `
  --config .\test-agents\1c-intake\agent-config.example.yaml `
  --settings-dir .\test-agents\1c-intake `
  --prompt "Собери первичную информацию по задаче K7-20486" `
  --home .\workflows\1c-dev\runs\manual-intake
```

Or via orchestrator:

```powershell
.\scripts\1c-dev-run.ps1 -Product k7 -IssueKey K7-20486 -Stage 1
```
