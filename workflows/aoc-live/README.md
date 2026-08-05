# Live AoC workflow example (Agent Kuibysheff ACP)

Demo of an **external singleton orchestrator** that speaks ACP over stdio to a
long-lived `agent_Kuibysheff acp` process, while talking to the real
[Advent of Code](https://adventofcode.com/) site.

Unlike the local regression harness (`scripts/aoc-eval.*`), this workflow:

1. Downloads the puzzle statement and personal input from AoC.
2. Builds a one-task bank + per-run home for the existing Referent AoC skill/MCP.
3. Starts **one** ACP child and drives `session/prompt` turns.
4. Submits the agent's final answer to AoC.
5. On a wrong answer, prompts the agent again with AoC feedback.
6. Stops at success or after **at most 5** full solve/submit iterations.

```text
Browser cookie (AOC_SESSION)
        │
        ▼
aoc-singleton.py  ──HTTP──►  adventofcode.com  (fetch / submit)
        │
        │ ACP stdio (long-lived)
        ▼
agent_Kuibysheff acp  ──MCP──►  mcp-aoc-tasks.js  (one-task bank)
        │
        └── home.run (python solution.py)
```

## Layout

```text
workflows/aoc-live/
  README.md            this file
  requirements.txt     Python deps (agent-client-protocol)
  aoc-singleton.py     CLI + retry loop
  aoc_http.py          AoC fetch/submit + verdict classification
  acp_bridge.py        ACP client (spawn + session/prompt)
  runtime.py           per-run home / bank / agent-config
  run.ps1 / run.sh     launchers
  test_smoke_offline.py  offline helper checks
  runs/                gitignored artifacts (report.json, homes, locks)
```

## Prerequisites

| Need | Notes |
|------|--------|
| `AOC_SESSION` | Session cookie from adventofcode.com (browser DevTools → Cookies) |
| Provider API key | From base config (`OPENAI_API_KEY` / `POLZA_API_KEY` / …); load via `.env` |
| `cargo build --release` | Produces `target/release/agent_Kuibysheff` |
| Node.js | For `mcp-aoc-tasks.js` |
| Python 3.10+ | Orchestrator + sandboxed `home.run` |
| `pip install -r workflows/aoc-live/requirements.txt` | ACP Python SDK |

Optional: stage AppContainer-friendly Python via `scripts/aoc-regression.ps1`
(writes `local/aoc-sandbox-runtime/python`). If missing, the host `python` is used.

## Quick start

```powershell
pip install -r .\workflows\aoc-live\requirements.txt
cargo build --release

# .env should contain AOC_SESSION=... and the provider key
.\workflows\aoc-live\run.ps1 -Year 2024 -Day 1 -Part 1
```

```bash
pip install -r ./workflows/aoc-live/requirements.txt
cargo build --release
export AOC_SESSION=...   # or use repo .env via import-dotenv.sh
./workflows/aoc-live/run.sh --year 2024 --day 1 --part 1
```

Direct Python:

```text
python workflows/aoc-live/aoc-singleton.py --year 2024 --day 1 --part 1
```

## CLI

| Flag | Default | Meaning |
|------|---------|---------|
| `--year` / `--day` / `--part` | (required / required / 1) | Puzzle coordinates |
| `--max-attempts` | 5 | Full iterations (hard-capped at 5) |
| `--config` | `test-agents/referent/agent-config.aoc.example.yaml` | Provider template |
| `--settings-dir` | `test-agents/referent` | Agent profile |
| `--home-root` | `workflows/aoc-live/runs` | Artifact root |
| `--agent-bin` | `target/release/agent_Kuibysheff[.exe]` | Override binary |
| `-v` | off | Debug logs (includes drained ACP stderr) |

Only **one** singleton process may run at a time (`runs/.aoc-singleton.lock`).

## Outcomes

| `report.json` status | Exit | Meaning |
|----------------------|------|---------|
| `correct` | 0 | AoC accepted the answer |
| `already_solved` | 0 | Puzzle part already completed for this account |
| `max_attempts_exhausted` | 1 | Five wrong (or empty) attempts |
| `wrong_level` | 1 | Submitting the wrong part |
| `auth_required` | 1 | Bad / missing `AOC_SESSION` |
| `unknown_submit` | 1 | Unrecognized AoC HTML |

Each attempt records `candidate`, ACP `stop_reason`, AoC `verdict`, hint
(`too high` / `too low`), and latency. Raw submit HTML is saved as
`submit-attempt-N.html` under the run directory.

## Smoke checks (manual)

1. **Happy path** — solved-or-easy day with valid `AOC_SESSION` and provider key;
   expect `status=correct` (or `already_solved`) and exit 0.
2. **Retry path** — temporarily break `solution.py` mid-run or use a hard puzzle;
   confirm attempt 2+ prompts include “REJECTED by Advent of Code” and
   `attempts.length <= 5`.
3. **Cap** — force wrong answers (e.g. mock / dry environment); confirm the
   process stops after 5 agent iterations and writes `max_attempts_exhausted`.
4. **Singleton** — start a second `run.ps1` while the first is busy; expect a
   lock error.

Offline helper check (no network / no agent):

```powershell
python .\workflows\aoc-live\test_smoke_offline.py
```

## Goals / boundaries

| In scope | Out of scope |
|----------|----------------|
| Example of ACP bridge orchestration | Replacing local `aoc-eval` regression |
| Live AoC fetch + submit | CI / automatic mass submission |
| ≤5 full agent iterations with feedback | Multi-day batch queues |
| Referent AoC skill + `home.run` | Changing the Rust ACP server |

Be a good AoC citizen: do not hammer the site, respect rate-limit waits, and
keep your session cookie private (`.env` is gitignored).
