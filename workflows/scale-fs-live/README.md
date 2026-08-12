# Scale-FS live LLM regression

Opt-in gate that runs a real `agent_Kuibysheff` + provider against a generated
corpus and asserts that `RunOutput.result` contains the planted needle.

Exercises:

| Task | Surface | Edge |
|------|---------|------|
| `search-many-01` | `local_tools.search_docs` / `read_file` | ~400 workspace files |
| `large-read-01` | `home.read` windows | ~100k chars; needle past default 50k window |
| `oversize-run-01` | `home.read` windows | ~900 KB; needle mid-file via `offset`/`next_offset` |

## Layout

```text
workflows/scale-fs-live/
  README.md
  corpus.py                 plant + verify corpus
  eval.py                   live harness
  assert_regression.py      gate on report.json
  test_corpus_offline.py    no-LLM unit checks
  runs/                     gitignored orchestration artifacts

test-agents/scale-fs-probe/ importable agent profile
local/scale-fs-bank.example/ sample task bank (copy to local/scale-fs-bank/)
local/scale-fs-runs/        gitignored reports + homes
```

## Prerequisites

- Provider API key via `api_key_env` (default `OPENAI_API_KEY`) in the environment or `.env`
- `cargo build --release` produces `agent_Kuibysheff`
- Python 3.10+ on `PATH` (corpus generator)

## Offline corpus check (no LLM)

```powershell
python .\workflows\scale-fs-live\test_corpus_offline.py
```

```bash
python3 ./workflows/scale-fs-live/test_corpus_offline.py
```

## Run (full bank)

```powershell
Copy-Item -Recurse .\local\scale-fs-bank.example .\local\scale-fs-bank
.\scripts\scale-fs-regression.ps1
# or
.\scripts\check.ps1 -ScaleFs
```

```bash
cp -R ./local/scale-fs-bank.example ./local/scale-fs-bank
./scripts/scale-fs-regression.sh
# or
./scripts/check.sh --scale-fs
```

Filter tasks:

```powershell
.\scripts\scale-fs-regression.ps1 -TaskId search-many-01,large-read-01
```

## Manual one-shot

```powershell
cargo build --release
$proj = "local\scale-fs-manual-project"
Remove-Item -Recurse -Force $proj -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $proj | Out-Null

.\target\release\agent_Kuibysheff.exe init scale-fs-probe --project-root $proj --force
.\target\release\agent_Kuibysheff.exe config --project-root $proj --agent scale-fs-probe `
  import --from .\test-agents\scale-fs-probe --force

# Plant corpus into project + a home under .kuibysheff (eval does this automatically).
python .\workflows\scale-fs-live\corpus.py --kind many_files --task-id search-many-01 --seed 42 --out $proj

# Edit protected agent-config.yaml: set workspace.root to the absolute project path,
# workspace.read: [corpus], and resolve python executable/runtime_read_roots for your host.
# Then:
.\target\release\agent_Kuibysheff.exe run --project-root $proj --agent scale-fs-probe `
  --home homes/manual --prompt "In the workspace corpus/, find the clearance code for Project Orion. Put only that code in result."
```

Prefer `eval.py` / `scale-fs-regression.*` for correct absolute workspace + python wiring.

## Scoring

Pass when `result` contains the planted `SF_NEEDLE_…` token (sidecar under
`local/scale-fs-runs/<run-id>/needles/`). Optional `expect.stop_reason` defaults to
requiring `goal_reached` when set in the bank JSON.
