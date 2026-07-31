#!/usr/bin/env bash
# Bash mirror of 1c-dev-run.ps1 (Windows-primary). Prefer the PowerShell script on Windows.
set -euo pipefail

PRODUCT=""
ISSUE_KEY=""
TASK_FILE=""
STAGE="all"
FROM_STAGE=""
APPROVE_PLAN=0
REQUIRE_TZ=0
REQUIRE_SEARX=0
BUILD_CFE=0
AGENT_BIN=""
RUNS_ROOT=""
RUN_ID=""
FORCE=0
REPO_ROOT=""

usage() {
  cat <<'EOF'
Usage: 1c-dev-run.sh --product demo [--issue-key PROJ-123 | --task-file path.md]
  [--stage all|1|2|3|4] [--from-stage N] [--approve-plan] [--require-tz]
  [--require-searx] [--build-cfe] [--agent-bin PATH] [--run-id ID] [--force]

On Windows use scripts/1c-dev-run.ps1. This shell script delegates to pwsh/powershell
when available; otherwise prints the equivalent PowerShell invocation.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --product) PRODUCT="$2"; shift 2 ;;
    --issue-key) ISSUE_KEY="$2"; shift 2 ;;
    --task-file) TASK_FILE="$2"; shift 2 ;;
    --stage) STAGE="$2"; shift 2 ;;
    --from-stage) FROM_STAGE="$2"; shift 2 ;;
    --approve-plan) APPROVE_PLAN=1; shift ;;
    --require-tz) REQUIRE_TZ=1; shift ;;
    --require-searx) REQUIRE_SEARX=1; shift ;;
    --build-cfe) BUILD_CFE=1; shift ;;
    --agent-bin) AGENT_BIN="$2"; shift 2 ;;
    --runs-root) RUNS_ROOT="$2"; shift 2 ;;
    --run-id) RUN_ID="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ -z "$PRODUCT" ]]; then
  echo "--product is required" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -z "$REPO_ROOT" ]]; then
  REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

PS1="$SCRIPT_DIR/1c-dev-run.ps1"
ARGS=( -Product "$PRODUCT" -Stage "$STAGE" -RepoRoot "$REPO_ROOT" )
[[ -n "$ISSUE_KEY" ]] && ARGS+=( -IssueKey "$ISSUE_KEY" )
[[ -n "$TASK_FILE" ]] && ARGS+=( -TaskFile "$TASK_FILE" )
[[ -n "$FROM_STAGE" ]] && ARGS+=( -FromStage "$FROM_STAGE" )
[[ -n "$AGENT_BIN" ]] && ARGS+=( -AgentBin "$AGENT_BIN" )
[[ -n "$RUNS_ROOT" ]] && ARGS+=( -RunsRoot "$RUNS_ROOT" )
[[ -n "$RUN_ID" ]] && ARGS+=( -RunId "$RUN_ID" )
[[ "$APPROVE_PLAN" -eq 1 ]] && ARGS+=( -ApprovePlan )
[[ "$REQUIRE_TZ" -eq 1 ]] && ARGS+=( -RequireTz )
[[ "$REQUIRE_SEARX" -eq 1 ]] && ARGS+=( -RequireSearx )
[[ "$BUILD_CFE" -eq 1 ]] && ARGS+=( -BuildCfe )
[[ "$FORCE" -eq 1 ]] && ARGS+=( -Force )

if command -v pwsh >/dev/null 2>&1; then
  exec pwsh -NoProfile -File "$PS1" "${ARGS[@]}"
elif command -v powershell.exe >/dev/null 2>&1; then
  exec powershell.exe -NoProfile -File "$PS1" "${ARGS[@]}"
elif command -v powershell >/dev/null 2>&1; then
  exec powershell -NoProfile -File "$PS1" "${ARGS[@]}"
else
  echo "PowerShell not found. Install pwsh or run scripts/1c-dev-run.ps1 on Windows." >&2
  echo "Would invoke: $PS1 ${ARGS[*]}" >&2
  exit 1
fi
