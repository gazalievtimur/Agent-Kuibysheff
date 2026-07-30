# 1c-analyst prompt example

```text
Подготовь утверждаемый план доработки в расширении для product=k7.

Read in/task_brief.md and in/product.json. Research CF. Use SearXNG only as supplement.
Write prd.md, architecture.md, tasks.md (labels bsl|metadata|cfe_packaging), cfe-scope.md,
workflow-state.md, manifest.json (apply_mode=none).

Return JSON only on every turn.
```

Orchestrator:

```powershell
.\scripts\1c-dev-run.ps1 -Product k7 -TaskFile .\path\to\task.md -Stage 2
```
