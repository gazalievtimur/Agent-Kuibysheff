# 1c-coder prompt example

```text
Реализуй утверждённые шаги кода для product=demo.
Read in/agreements-protocol.md and in/agreements.md first (identifiers are literals).
Read in/tasks.md. Implement only bsl/metadata into out/src/.
Write code-report.md (verification table), files-index.md, manifest.json (apply_mode=none).
Return JSON only on every turn.
```

```powershell
.\scripts\1c-dev-run.ps1 -Product demo -IssueKey PROJ-123 -FromStage 3 -ApprovePlan
```
