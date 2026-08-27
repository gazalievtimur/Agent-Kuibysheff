# 1c-intake

Stage 1 of the 1C Kuibysheff conveyor: Jira/Confluence → `task_brief.md`.
Orchestrator entrypoints: `scripts/1c-dev-run.ps1` / `scripts/1c-dev-scaffold-project.ps1`
(full copy-unit under local `workflows/1c-dev/` when restored from git history).

## Dependencies

- `uvx` + `mcp-atlassian`
- Env: `JIRA_URL`, `JIRA_PERSONAL_TOKEN` (or username/token pair), `CONFLUENCE_URL`, `CONFLUENCE_PERSONAL_TOKEN`
- Vision-capable model recommended for screenshots

## Skip

When the orchestrator receives `-TaskFile`, this agent is not run.
