# Local AoC evaluation data

This directory holds **local-only** Advent of Code evaluation assets.

| Path | In git? | Purpose |
| --- | --- | --- |
| `aoc-bank.example/` | yes | Schema sample and one toy task |
| `aoc-bank/` | **no** | Your real task bank (`id`, `text`, `input`, `expected`) |
| `aoc-runs/` | **no** | Per-run homes and `report.json` from the harness |
| `aoc-sandbox-runtime/` | **no** | Staged Python tree for AppContainer ACL grants |

## Setup

Windows:

```powershell
Copy-Item -Recurse .\local\aoc-bank.example .\local\aoc-bank
# Edit/replace JSON files under local/aoc-bank with real puzzles.
```

Linux:

```bash
cp -R ./local/aoc-bank.example ./local/aoc-bank
# Edit/replace JSON files under local/aoc-bank with real puzzles.
```

Optional:

```powershell
$env:AOC_BANK_DIR = (Resolve-Path .\local\aoc-bank).Path
```

```bash
export AOC_BANK_DIR="$(pwd)/local/aoc-bank"
```

Requirements:

- Node.js (for `mcp-aoc-tasks.js`)
- Python on `PATH` (agents use `home.run` with `program=python`)
- Provider API key from the chosen config (`OPENAI_API_KEY` by default)

## Run eval (Referent)

Windows:

```powershell
.\scripts\aoc-eval.ps1
.\scripts\aoc-eval.ps1 -TaskId 2024-01-1
```

Linux:

```bash
chmod +x ./scripts/*.sh
./scripts/aoc-eval.sh
./scripts/aoc-eval.sh --task-id 2024-01-1
```

### Regression gate

Live agent evals are opt-in local gates (not PR CI):

- **AoC:** `scripts/check.ps1 -Aoc` / `RUN_AOC=1`, or Linux `scripts/check.sh` (default; `--skip-aoc` to omit)
- **SWE-bench (one Verified instance):** `scripts/check.ps1 -Swebench` / `RUN_SWEBENCH=1`, or `./scripts/check.sh --swebench`

```powershell
$env:POLZA_API_KEY = "..."   # or set provider api_key_env / .env
.\scripts\check.ps1                    # fmt/clippy/deny/cargo test (no live evals)
.\scripts\check.ps1 -Aoc               # + AoC regression
.\scripts\aoc-regression.ps1           # AoC-only
.\scripts\check.ps1 -Swebench          # + SWE-bench regression (Docker + LLM)
.\scripts\swebench-regression.ps1      # SWE-bench-only (sympy__sympy-20590)
.\scripts\swebench-regression-linux-docker.ps1  # same gate as Linux ELF via Docker
.\scripts\check.ps1 -SkipDeny          # skip cargo deny (supply-chain)
```

```bash
export POLZA_API_KEY="..."   # or set provider api_key_env / .env
./scripts/check.sh                     # includes AoC by default
./scripts/aoc-regression.sh            # AoC-only
./scripts/check.sh --skip-aoc          # fmt/clippy/deny/cargo test only
./scripts/check.sh --swebench          # + SWE-bench regression (Docker + LLM)
./scripts/swebench-regression.sh       # SWE-bench-only (sympy__sympy-20590)
./scripts/check.sh --skip-deny         # skip cargo deny (supply-chain)
```

Supply-chain policy lives in `deny.toml`. Install once: `cargo install --locked cargo-deny`.

#### AoC gate requirements

- `local/aoc-bank/` with at least one task JSON
- `agent-config.local.yaml` (preferred) or `test-agents/referent/agent-config.aoc.example.yaml`
- API key env var from that config
- Node.js + Python on `PATH` (resolved into sandboxed `home.run`)
- OS sandbox available (Windows AppContainer / Linux namespaces)

The AoC harness:

1. Builds a fresh `target/release` agent (`aoc-regression.ps1` / `aoc-regression.sh`)
2. Loads tasks from `local/aoc-bank`
3. Writes a per-run config with fail-closed `access` (python alias + runtime roots)
4. Imports `test-agents/referent` into a protected profile, then runs
   `agent_Kuibysheff run --project-root … --agent …` once per task
5. Compares `RunOutput.result` to `expected`
6. Writes `local/aoc-runs/<run-id>/report.json`

#### SWE-bench gate requirements

- Docker Desktop / Linux engine (x86_64 images)
- `pip install -r workflows/swebench-verified/requirements.txt`
- `agent-config.local.yaml` (preferred) or `test-agents/swebench-solver/agent-config.example.yaml`
- Provider API key via `api_key_env`
- Disk/time for the official instance image

The SWE-bench harness (`swebench-regression.*`):

1. Checks Docker Linux + Python deps + API key (no gold harness)
2. Builds `target/release`
3. Runs `generate → grade → report` for fixed instance `sympy__sympy-20590`
4. Asserts `harness_resolved=true` via `workflows/swebench-verified/assert_regression.py`
5. Prints UX summary (stop_reason, elapsed, usage/cost) even on failure

**Windows native** uses the PE worker + `harness_bootstrap.py` (LF for grade scripts).
**Linux native** uses `./scripts/swebench-regression.sh`.
**Linux ELF from Windows** (no WSL): `.\scripts\swebench-regression-linux-docker.ps1` —
builds inside `rust:1-bookworm`, isolates `CARGO_TARGET_DIR`, passes `--agent-bin`,
and sets `KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1` for nested Docker without `clone3`.

Pitfalls and layout details:
[workflows/swebench-verified/README.md](../workflows/swebench-verified/README.md)
(section *Regression gate*).

On Linux the AoC harness uses the host `python3` directly (namespace mounts cover
runtime roots). See also [crates/sandbox-linux/TESTING.md](../crates/sandbox-linux/TESTING.md)
for userns / AppArmor notes on lab hosts.

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
