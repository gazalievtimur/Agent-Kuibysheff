# 1c-analyst prompt example

```text
Подготовь утверждаемый план доработки в расширении для product=demo.

Read in/agreements-protocol.md and in/agreements.json first (identifiers are literals).
Read in/task_brief.md and in/product.json. Research CF. Use SearXNG only as supplement.
Write agreements.md (verbatim identifiers) first, then prd.md, architecture.md,
tasks.md (labels bsl|metadata|cfe_packaging; repeat gate procedure verbatim),
cfe-scope.md, workflow-state.md, manifest.json (apply_mode=none).
Include a verification table before done=true.

Return JSON only on every turn.
```

Orchestrator:

```powershell
.\scripts\1c-dev-run.ps1 -Product demo -TaskFile .\path\to\task.md -Stage 2
```
