# 1c-coder prompt example

```text
Реализуй утверждённые шаги кода для product=k7.
Read in/tasks.md. Implement only bsl/metadata into out/src/.
Write code-report.md, files-index.md, manifest.json (apply_mode=none).
Return JSON only on every turn.
```

```powershell
.\scripts\1c-dev-run.ps1 -Product k7 -IssueKey K7-20486 -FromStage 3 -ApprovePlan
```
