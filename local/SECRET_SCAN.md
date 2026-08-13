# Secret scan notes (local)

Gitleaks report (redacted): `local/gitleaks-report.json` (gitignored).

## Documented false positives / intentional placeholders

Manual `rg` hits that are **not** live secrets:

- `.env.example`, README, workflow docs — `api_key_env` / `your_api_key` / `sk-...` placeholders
- `agent-config.example.yaml` — commented inline `api_key` example (inline keys are rejected by config safety)
- Test fixtures using the word `secret` / `CancellationToken` / canary `token` fields
- Regexes in AoC/SWE/security scripts that *detect* inline `api_key:` for fail-closed checks

Re-run before public Go:

```powershell
gitleaks git . --redact --no-banner -r local/gitleaks-report.json
```
