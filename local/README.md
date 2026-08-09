# Local AoC evaluation data

This directory holds **local-only** Advent of Code evaluation assets and
security-sandbox regression data.

| Path | In git? | Purpose |
| --- | --- | --- |
| `aoc-bank.example/` | yes | Schema sample and one toy task |
| `aoc-bank/` | **no** | Your real task bank (`id`, `text`, `input`, `expected`) |
| `aoc-runs/` | **no** | Per-run homes and `report.json` from the harness |
| `aoc-sandbox-runtime/` | **no** | Staged Python tree for AppContainer ACL grants |
| `security-bank.example/` | yes | Adversarial prompt bank sample (containment scoring) |
| `security-bank/` | **no** | Working security bank copy |
| `security-runs/` | **no** | Security eval homes + `report.json` |
| `security-host-canary/` | **no** | Native (non-Docker) host canary directory |

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

AoC eval is part of the normal local quality gate:

- Windows: every `scripts/check.ps1` (unless `-SkipAoc`)
- Linux: every `scripts/check.sh` (unless `--skip-aoc`)

Security sandbox LLM regression is opt-in:

- Windows: `scripts/check.ps1 -Security` / `RUN_SECURITY=1`
- Linux: `./scripts/check.sh --security` / `RUN_SECURITY=1`

```powershell
$env:POLZA_API_KEY = "..."   # or set provider.api_key / .env
.\scripts\check.ps1
.\scripts\aoc-regression.ps1              # AoC-only
.\scripts\security-regression.ps1         # security-only (Docker lab on Windows)
.\scripts\check.ps1 -Security             # + security containment regression
.\scripts\check.ps1 -SkipAoc              # fmt/clippy/deny/cargo test only
.\scripts\check.ps1 -SkipDeny             # skip cargo deny (supply-chain)
```

```bash
export POLZA_API_KEY="..."   # or set provider.api_key / .env
./scripts/check.sh
./scripts/aoc-regression.sh              # AoC-only
./scripts/security-regression.sh         # security-only
./scripts/check.sh --security            # + security containment regression
./scripts/check.sh --skip-aoc            # fmt/clippy/deny/cargo test only
./scripts/check.sh --skip-deny           # skip cargo deny (supply-chain)
```

Supply-chain policy lives in `deny.toml`. Install once: `cargo install --locked cargo-deny`.

Requirements for the AoC gate:

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

#### Security sandbox gate

- Copy `local/security-bank.example/` → `local/security-bank/`
- Config: `test-agents/security-probe/` (RUB cost variants: `agent-config.terra-pro*.example.yaml`)
- Pass = containment (canaries intact); optional `--require-cost-limit` for budget stop
- Docker lab: `scripts/security-regression-linux-docker.*` (no docker.sock; sandbox probe required)
- Details: [workflows/security-sandbox/README.md](../workflows/security-sandbox/README.md)

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
