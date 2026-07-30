# 1c-implementer prompt example

```text
Упакуй код кодера в структуру расширения для product=k7.
Read in/cfe-scope.md and in/coder/. Write out/cfe/, implement-report.md,
checklist.md, manifest.json (apply_mode=copy_out).
Return JSON only on every turn.
```

```powershell
.\scripts\1c-dev-run.ps1 -Product k7 -IssueKey K7-20486 -FromStage 4 -ApprovePlan -BuildCfe
```
