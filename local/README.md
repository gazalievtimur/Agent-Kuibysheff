# Local evaluation data

Local-only banks and run artifacts for **in-tree** live regressions
(security, Scale-FS, A2A). AoC / SWE-bench / 1C live evals live in separate
example repositories — see the root [README](../README.md#examples--live-regressions).

Remaining gitignored `workflows/` copy-units (security, scale-fs, 1c-dev) can be
restored from git history when needed:

```bash
git checkout <commit-before-untrack> -- workflows
```

| Path | In git? | Purpose |
| --- | --- | --- |
| `security-bank.example/` | yes | Adversarial prompt bank sample (containment scoring) |
| `security-bank/` | **no** | Working security bank copy |
| `security-runs/` | **no** | Security eval homes + `report.json` |
| `security-host-canary/` | **no** | Native (non-Docker) host canary directory |
| `scale-fs-bank.example/` | yes | Scale-FS corpus tasks (many files / large / oversize) |
| `scale-fs-bank/` | **no** | Working Scale-FS bank copy |
| `scale-fs-runs/` | **no** | Scale-FS eval homes + `report.json` |
| `a2a-bank.example/` | yes | A2A live tasks (card, bearer, send) |
| `a2a-bank/` | **no** | Working A2A bank copy |
| `a2a-runs/` | **no** | A2A eval projects + `report.json` |
| `SECRET_SCAN.md` | yes | Notes for gitleaks / placeholder false positives |
| `gitleaks-report.json` | **no** | Redacted local gitleaks output |

## External example repos

| Suite | Repo |
| --- | --- |
| AoC + AoC-live | https://github.com/gazalievtimur/kuibysheff-aoc |
| SWE-bench Verified | https://github.com/gazalievtimur/kuibysheff-swebench |
| 1C CF/CFE live | https://github.com/gazalievtimur/kuibysheff-1c-live |

From this agent checkout, `check -Aoc` / `check -Swebench` delegate to sibling
clones (or `KUIBYSHEFF_AOC_ROOT` / `KUIBYSHEFF_SWEBENCH_ROOT`) with
`KUIBYSHEFF_SRC` pointing here.

## In-tree opt-in gates

```powershell
$env:OPENAI_API_KEY = "..."
.\scripts\check.ps1                    # fmt/clippy/deny/cargo test (no live evals)
.\scripts\check.ps1 -Security
.\scripts\check.ps1 -ScaleFs
.\scripts\a2a-regression.ps1
```

```bash
export OPENAI_API_KEY="..."
./scripts/check.sh
./scripts/check.sh --security
./scripts/check.sh --scale-fs
./scripts/a2a-regression.sh
```

See each bank's `*.example/README.md` for schemas.
