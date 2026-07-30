# 1c-intake

Stage 1 of the [1C Kuibyshev workflow](../../workflows/1c-dev/README.md): Jira/Confluence → `task_brief.md`.

## Dependencies

- `uvx` + `mcp-atlassian`
- Env: `JIRA_URL`, `JIRA_PERSONAL_TOKEN` (or username/token pair), `CONFLUENCE_URL`, `CONFLUENCE_PERSONAL_TOKEN`
- Vision-capable model recommended for screenshots

## Skip

When the orchestrator receives `-TaskFile`, this agent is not run.
