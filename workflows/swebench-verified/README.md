# SWE-bench Verified workflow

Two-phase orchestrator that runs `agent_Kuibysheff` on
[`SWE-bench/SWE-bench_Verified`](https://huggingface.co/datasets/SWE-bench/SWE-bench_Verified)
tasks inside isolated Docker containers, then grades patches with the **official**
`swebench.harness.run_evaluation` harness.

The Rust CLI stays a stateless worker. This workflow owns Docker lifecycle, patch
extraction, resume, grading, and reports.

## Layout

```text
workflows/swebench-verified/          copy unit (portable)
  swebench.py                         CLI entry (preflight|generate|grade|report|run)
  run.ps1 / run.sh                    local launchers
  runtime.py                          runs layout, config render, patch extract, report
  swebench_adapter.py                 pinned dataset + safe projection + image keys
  docker_workspace_mcp.py             fail-closed workspace.* MCP (one container)
  solver/                             bundled solver profile (defaults)
  requirements.txt                    pinned swebench / docker / mcp
  test_smoke_offline.py               offline unit checks
  runs/                               gitignored artifacts

scripts/swebench-verified-run.*       thin forwards to the copy unit (monorepo UX)
```

External dependencies (not part of the copy unit): `agent_Kuibysheff` on PATH or
`--agent-bin`, Docker Linux engine, provider API key, Python deps from
`requirements.txt`.

## Requirements (MVP)

- Docker Desktop with **Linux** engine (x86_64 images)
- Python 3.10+
- Release binary: `cargo build --release` → `target/release/agent_Kuibysheff`
- Provider API key via `.env` / env named by `provider.api_key_env` (inline `api_key` rejected)
- Disk/time for SWE-bench instance images (large)

Pinned Python deps:

```powershell
pip install -r workflows/swebench-verified/requirements.txt
```

## Commands

```powershell
# Launchers forward to swebench.py
.\scripts\swebench-verified-run.ps1 preflight
.\scripts\swebench-verified-run.ps1 generate --instance-id sympy__sympy-20590
.\scripts\swebench-verified-run.ps1 grade --run-id <id>
.\scripts\swebench-verified-run.ps1 report --run-id <id>
.\scripts\swebench-verified-run.ps1 run --instance-id sympy__sympy-20590 --run-id demo1
```

```bash
./scripts/swebench-verified-run.sh preflight
./scripts/swebench-verified-run.sh generate --instance-id sympy__sympy-20590
./scripts/swebench-verified-run.sh run --slice 0:1 --run-id demo1
```

Common flags:

| Flag | Meaning |
| --- | --- |
| `--instance-id ID` | One or more tasks (repeatable) |
| `--slice START:END` | Deterministic dataset range |
| `--workers N` | Parallel generate/grade workers |
| `--run-id ID` | Explicit run identity |
| `--resume` | Skip terminal `ok` instances with a valid patch |
| `--agent` | Agent id (default `swebench-solver`) |
| `--home` | Relative under `.kuibysheff/` (default `homes/work`) |
| `--agent-bin` | Override binary path |
| `--config`, `--settings-dir` | Import/render sources only (never passed to agent) |

Default: **one model attempt** per instance. Only infrastructure failures are
retried via `--resume`. Do not cherry-pick the best of N without marking the
run as a separate experiment.

## Architecture

1. **Generate** — start a clean task container (`network_disabled`, no host
   mounts/secrets), bind `docker_workspace_mcp.py` to that container id, run
   one-shot `agent_Kuibysheff run`, extract `git diff` (including untracked via
   `git add -N`), validate with `git apply --check`, write per-instance status.
2. **Grade** — call official harness on `predictions.jsonl` with a **new**
   container; never reuse the generation container for scoring.
3. **Report** — merge resolved flags, `RunOutput.usage` (cost status preserved;
   missing prices are never treated as zero), and provenance into `report.json`.

Oracle fields (`patch`, `test_patch`, `FAIL_TO_PASS`, `PASS_TO_PASS`,
`hints_text`) are never projected into the prompt or MCP.

## Artifacts

```text
runs/<run-id>/
  manifest.json
  predictions.jsonl
  report.json
  harness/                  # copied upstream logs when available
  instances/<instance-id>/
    status.json
    model.patch
    run-output.json
    agent.stderr.txt
    provenance.json
    .kuibysheff/
      protected/agents/swebench-solver/   # imported profile + generated config
      homes/work/                         # relative --home
```

Each instance directory is the `--project-root`. The agent is invoked as:

```text
agent_Kuibysheff run --project-root <instance_dir> --agent swebench-solver --home homes/work ...
```

## Security

- API keys stay on the host agent process; task containers get no provider env.
- MCP cannot list/create/remove Docker resources or retarget containers.
- Paths resolve only under `/testbed`; absolute paths, `..`, and symlink escapes
  are rejected.
- Generation images do not receive gold patches or eval scripts from this
  workflow.

## Testing

Offline (no Docker/LLM):

```powershell
python workflows/swebench-verified/test_smoke_offline.py
```

Optional Docker smoke (fixture alpine/git image, no LLM):

```powershell
python workflows/swebench-verified/test_docker_smoke.py
```

Manual checklist:

1. `preflight` (includes gold harness on `sympy__sympy-20590`; use `--skip-gold` to omit)
2. Real-model `generate` on one instance → `grade` → `report`
3. Confirm oracle fields absent from prompt/MCP payloads
4. Confirm API key absent from task-container environment
5. Untracked new file appears in `model.patch`
6. Interrupt a batch and continue with `--resume`
7. Two parallel instances (`--workers 2`) without mixed configs/logs

Full Verified-500 is **not** part of PR CI.

## MVP boundaries

In scope: local Docker Linux x86_64, Verified `test` split, one solver profile,
one-shot `run`, resumable generation, official local grading, JSON reports.

Out of scope: Modal/cloud eval, ARM-native images, ACP multi-turn after official
tests, multi-agent review, best-of-N, leaderboard submission, PR CI full runs.
