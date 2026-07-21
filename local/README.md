# Local AoC evaluation data

This directory holds **local-only** Advent of Code evaluation assets.

| Path | In git? | Purpose |
| --- | --- | --- |
| `aoc-bank.example/` | yes | Schema sample and one toy task |
| `aoc-bank/` | **no** | Your real task bank (`id`, `text`, `input`, `expected`) |
| `aoc-runs/` | **no** | Per-run homes and `report.json` from the harness |
| `aoc-sandbox-runtime/` | **no** | Staged Python tree for AppContainer ACL grants |

## Setup

```powershell
Copy-Item -Recurse .\local\aoc-bank.example .\local\aoc-bank
# Edit/replace JSON files under local/aoc-bank with real puzzles.
```

Optional:

```powershell
$env:AOC_BANK_DIR = (Resolve-Path .\local\aoc-bank).Path
```

Requirements:

- Node.js (for `mcp-aoc-tasks.js`)
- Python on `PATH` (agents use `home.run` with `program=python`)
- Provider API key from the chosen config (`OPENAI_API_KEY` by default)

## Run eval (Referent)

```powershell
.\scripts\aoc-eval.ps1
.\scripts\aoc-eval.ps1 -TaskId 2024-01-1
```

### Regression gate

AoC eval is part of the normal local quality gate and runs on every
`scripts/check.ps1` invocation (unless `-SkipAoc`):

```powershell
$env:POLZA_API_KEY = "..."   # or set provider.api_key / .env
.\scripts\check.ps1
.\scripts\aoc-regression.ps1              # AoC-only
.\scripts\check.ps1 -SkipAoc              # fmt/clippy/cargo test only
```

Requirements for the gate:

- `local/aoc-bank/` with at least one task JSON
- `agent-config.local.yaml` (preferred) or `test-agents/referent/agent-config.aoc.example.yaml`
- API key env var from that config
- Node.js + Python on `PATH` (resolved into sandboxed `home.run`)
- OS sandbox available (Windows AppContainer / Linux namespaces)

The harness:

1. Builds a fresh `target/release` agent (`aoc-regression.ps1`)
2. Loads tasks from `local/aoc-bank`
3. Writes a per-run config with fail-closed `access` (python alias + runtime roots)
4. Runs `agent_Kuibyshev` once per task with `--settings-dir test-agents/referent`
5. Compares `RunOutput.result` to `expected`
6. Writes `local/aoc-runs/<run-id>/report.json`

Each AoC task run writes agent logs under:

```text
local/aoc-runs/<run-id>/<task-id>/logs/
  agent.trace.log
  ai_usage.jsonl
  mcp_usage.jsonl
  chat_history.json
```

Paths are also returned in `RunOutput.logs` inside `agent.stdout.json` and in
`report.json` per task.
