# 1c-implementer prompt example

```text
Упакуй код кодера в структуру расширения для product=demo.
Read in/agreements-protocol.md and in/agreements.md first (identifiers are literals).
Read in/cfe-scope.md and in/coder/. Write out/cfe/, copy in/cfe-tests to out/cfe-tests
without rewriting tests, implement-report.md (verification table),
checklist.md, manifest.json (apply_mode=copy_out).
Return JSON only on every turn.
```

```powershell
.\scripts\1c-dev-run.ps1 -Product demo -IssueKey PROJ-123 -FromStage 4 -ApprovePlan -BuildCfe
```
