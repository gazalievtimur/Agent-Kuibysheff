# Live test run summary — 2026-09-23

Branch: `main` (post Dependabot merges + rustls 0.23.45)
Host: Windows (Docker Desktop for Security / SWE-bench)
Provider: Polza AI (`POLZA_API_KEY`)

| Suite | Status | Notes |
| --- | --- | --- |
| **A2A** | **pass** (3/3) | `local/a2a-runs/20260923-151509/report.json` |
| **Scale-FS** | **pass** (3/3) | Updated local `workflows/scale-fs-live` to use `kbshff.exe` |
| **Security** | **pass** (8/8) | Docker lab; `workflows/security-sandbox` → `kbshff` |
| **AoC** | **pass** (2/2) | Offline bank via `kuibysheff-aoc` |
| **SWE-bench** | **pass** (resolved) | `sympy__sympy-20590` |
| **1C live** | **pass** (1/1) | `cfe-qty-check-01`; code-index daemon on `C:\MCP\code-index` |

## Artifacts

- A2A: `local/test-run-a2a.log`
- Scale-FS: `local/test-run-scale-fs.log` / `local/scale-fs-runs/20260923-151709/report.json`
- Security: `local/test-run-security.log`
- AoC: `local/test-run-aoc.log`
- SWE-bench: `local/test-run-swebench.log`
- 1C live: `C:\Git\kuibysheff-1c-live\harness\runs\20260923-155056\report.json`
