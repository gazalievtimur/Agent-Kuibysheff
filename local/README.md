# Local AoC evaluation data

This directory holds **local-only** Advent of Code evaluation assets.

| Path | In git? | Purpose |
| --- | --- | --- |
| `aoc-bank.example/` | yes | Schema sample and one toy task |
| `aoc-bank/` | **no** | Your real task bank (`id`, `text`, `input`, `expected`) |
| `aoc-runs/` | **no** | Per-run homes and `report.json` from the harness |

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

The harness:

1. Loads tasks from `local/aoc-bank`
2. Runs `agent_Kuibyshev` once per task with `--settings-dir test-agents/referent`
3. Compares `RunOutput.result` to `expected`
4. Writes `local/aoc-runs/<run-id>/report.json`

MCP `aoc` never returns `expected` — only the harness reads that field from disk.
