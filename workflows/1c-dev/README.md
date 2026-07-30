# 1C development workflow (Agent Kuibyshev)

Four-stage external orchestrator around the stateless `agent_Kuibyshev` worker:

1. **1c-intake** — Jira/Confluence → `task_brief.md` (skipped with `-TaskFile`)
2. **1c-analyst** — brief + CF research (+ SearXNG) → approvable plan
3. **1c-coder** — approved `tasks.md` → `out/src` sources
4. **1c-implementer** — package → `out/cfe` apply-ready tree

Human gate between plan and coder: `-ApprovePlan` or `artifacts/plan/APPROVED`.

## Quick start

```powershell
# From repo root; ensure .env has provider + Atlassian tokens as needed
.\scripts\1c-dev-run.ps1 -Product k7 -IssueKey K7-20486 -Stage 1

# Or skip intake with an operator-provided task file:
.\scripts\1c-dev-run.ps1 -Product k7 -TaskFile .\path\to\task.md -Stage 2

# After reviewing artifacts/plan, continue:
.\scripts\1c-dev-run.ps1 -Product k7 -IssueKey K7-20486 -RunId <id> -FromStage 3 -ApprovePlan

# Full conveyor after approval; copy/build CFE into the product task dir:
.\scripts\1c-dev-run.ps1 -Product k7 -IssueKey K7-20486 -ApprovePlan -ApplyOut -BuildCfe
```

CLI notes: `-Stage` / `-TaskFile` / `-Force` / `-ApplyOut` are aliases for `-WorkflowStage` / `-TaskFilePath` / `-ForceRerun` / `-DoApplyOut`.

Bash/WSL: `./scripts/1c-dev-run.sh --product k7 --issue-key K7-20486 --stage 1` (delegates to PowerShell).

## Layout

```text
workflows/1c-dev/
  schema/           artifact contracts per stage
  products/         k7.yaml, zup.yaml.example
  prompts/          stage prompt templates
  adapters/k7/      prepare-home, validate, apply-out
  runs/             gitignored run homes + report.json
```

Agent settings live under `test-agents/1c-{intake,analyst,coder,implementer}/`.

## Product adapter

Edit `products/k7.yaml` paths for your machine. ZUP: copy `products/zup.yaml.example` → `zup.yaml`.

## Prerequisites (K7)

| Stage | Needs |
|-------|--------|
| 1 | `uvx` + mcp-atlassian; `JIRA_*` / `CONFLUENCE_*` env |
| 2 | 1c-sntx-sem, code-index, SearXNG MCP (`searxngUrl`); optional conf-doc |
| 3–4 | Same code research MCPs; optional bsl-language-server |
| apply | K7 `cfe-task-extension.ps1` when using `-BuildCfe` |

SearXNG: see [test-agents/searxng](../../test-agents/searxng/). Without it, stage 2 warns and continues unless `-RequireSearx`.

## Goals / boundaries

See schema files and each agent's `master_prompt.md`. Summary:

| Stage | Goal | Out of scope |
|-------|------|--------------|
| intake | Verifiable brief | Code analysis, plan, Jira writes |
| analyst | Approvable CFE plan | Source edits, CFE build |
| coder | `out/src` for bsl/metadata tasks | CFE packaging, scope creep |
| implementer | `out/cfe` packaging | New business logic, auto IB load |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success for requested stages |
| 2 | Waiting on plan approval gate |
| other | Validation / agent / apply failure |
