# Local AoC evaluation data

This directory holds **local-only** Advent of Code evaluation assets,
security-sandbox regression data, and Scale-FS live regression assets.

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
| `scale-fs-bank.example/` | yes | Scale-FS corpus tasks (many files / large / oversize) |
| `scale-fs-bank/` | **no** | Working Scale-FS bank copy |
| `scale-fs-runs/` | **no** | Scale-FS eval homes + `report.json` |
| `SECRET_SCAN.md` | yes | Notes for gitleaks / placeholder false positives |
| `gitleaks-report.json` | **no** | Redacted local gitleaks output |

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

- **AoC:** opt-in on both OS — `scripts/check.ps1 -Aoc` / `RUN_AOC=1`, or Linux `scripts/check.sh --aoc` / `RUN_AOC=1`
- **SWE-bench (one Verified instance):** `scripts/check.ps1 -Swebench` / `RUN_SWEBENCH=1`, or `./scripts/check.sh --swebench`
- **Security sandbox (LLM containment):** `scripts/check.ps1 -Security` / `RUN_SECURITY=1`, or `./scripts/check.sh --security`
- **Scale-FS (many files / large reads):** `scripts/check.ps1 -ScaleFs` / `RUN_SCALE_FS=1`, or `./scripts/check.sh --scale-fs`

Security sandbox LLM regression is opt-in:

- Windows: `scripts/check.ps1 -Security` / `RUN_SECURITY=1`
- Linux: `./scripts/check.sh --security` / `RUN_SECURITY=1`

```powershell
$env:POLZA_API_KEY = "..."   # or set provider api_key_env / .env
.\scripts\check.ps1                    # fmt/clippy/deny/cargo test (no live evals)
.\scripts\check.ps1 -Aoc               # + AoC regression
.\scripts\aoc-regression.ps1           # AoC-only
.\scripts\check.ps1 -Swebench          # + SWE-bench regression (Docker + LLM)
.\scripts\swebench-regression.ps1      # SWE-bench-only (sympy__sympy-20590)
.\scripts\swebench-regression-linux-docker.ps1  # same gate as Linux ELF via Docker
.\scripts\check.ps1 -Security          # + security sandbox LLM regression (Docker lab on Windows)
.\scripts\security-regression.ps1      # security-only (forwards to Linux Docker on Windows)
.\scripts\check.ps1 -ScaleFs           # + Scale-FS live corpus regression
.\scripts\scale-fs-regression.ps1      # Scale-FS-only
.\scripts\check.ps1 -SkipDeny          # skip cargo deny (supply-chain)
```

```bash
export POLZA_API_KEY="..."   # or set provider api_key_env / .env
./scripts/check.sh                     # fmt/clippy/deny/tests/portability (AoC opt-in)
./scripts/check.sh --aoc               # + live AoC regression
./scripts/aoc-regression.sh            # AoC-only
./scripts/check.sh --swebench          # + SWE-bench regression (Docker + LLM)
./scripts/swebench-regression.sh       # SWE-bench-only (sympy__sympy-20590)
./scripts/check.sh --security          # + security sandbox LLM regression
./scripts/security-regression.sh       # security-only (native Linux userns)
./scripts/security-regression-linux-docker.sh  # security lab via Docker
./scripts/check.sh --scale-fs          # + Scale-FS live corpus regression
./scripts/scale-fs-regression.sh       # Scale-FS-only
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

#### Security sandbox gate requirements

- `local/security-bank/` (copy from `local/security-bank.example/`)
- `agent-config.local.yaml` (preferred) or `test-agents/security-probe/agent-config.example.yaml`
- Provider API key via `api_key_env`
- Working OS sandbox (**required** — never `KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP`)
- On Windows: Docker Desktop for the Linux privileged lab (`security-regression-linux-docker.ps1`)

The security harness:

1. Plants canaries (sibling FS, protected store, host `/canary`, network listener)
2. Runs a real LLM against adversarial prompts (`test-agents/security-probe`)
3. Passes only if containment holds (canaries intact, no token exfil)
4. Writes `local/security-runs/<run-id>/report.json`

Details: [workflows/security-sandbox/README.md](../workflows/security-sandbox/README.md).

#### Scale-FS live gate requirements

- `local/scale-fs-bank/` (auto-copied from `local/scale-fs-bank.example/` by the script)
- `agent-config.local.yaml` (preferred) or `test-agents/scale-fs-probe/agent-config.example.yaml`
- Provider API key via `api_key_env`
- Python on `PATH` (corpus generator; optional for sandboxed `home.run`)
- OS sandbox available if the profile still allowlists `home.run`

The Scale-FS harness:

1. Builds `target/release` agent
2. Plants a generated corpus (many files / large / oversize) per task
3. Imports `test-agents/scale-fs-probe` and rewrites workspace paths
4. Runs a real LLM; asserts planted `SF_NEEDLE_…` appears in `RunOutput.result`
5. Writes `local/scale-fs-runs/<run-id>/report.json`

Details: [workflows/scale-fs-live/README.md](../workflows/scale-fs-live/README.md).

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
