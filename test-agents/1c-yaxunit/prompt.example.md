# 1c-yaxunit prompt example

```text
Write YAxUnit tests for the approved plan (product=sklad).
Read in/agreements-protocol.md, in/agreements.json, in/agreements.md first
(identifiers are literals; do not rename the gate procedure).
Read in/docs/ (public YAxUnit snapshot). Read in/expect.json.
Do not implement the feature. Write out/tests/, out/cfe-tests/, test-report.md
(with a verification table), manifest.json (apply_mode=none).
Cite docs URLs in test-report.md. Return JSON only on every turn.
```

Live eval (copy-unit):

```powershell
.\workflows\1c-live\run.ps1 -TaskId cfe-qty-check-01
```
