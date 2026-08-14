#!/usr/bin/env bash
# Opt-in SWE-bench Verified capability regression (one fixed instance).
#
# Preflight-lite (Docker Linux, Python deps, API key) → release build →
# generate+grade+report for sympy__sympy-20590 → assert harness_resolved.
# Intended for: ./scripts/check.sh --swebench / RUN_SWEBENCH=1 (not PR CI).
#
# Usage:
#   ./scripts/swebench-regression.sh
#   ./scripts/swebench-regression.sh --config path/to/config.yaml
#   ./scripts/swebench-regression.sh --instance-id sympy__sympy-20590
#   ./scripts/swebench-regression.sh --agent-bin /path/to/kbshff
#
# On Windows hosts without WSL/Linux toolchain, use:
#   ./scripts/swebench-regression-linux-docker.ps1
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# shellcheck source=import-dotenv.sh
source "$SCRIPT_DIR/import-dotenv.sh"
import_dotenv "$REPO_ROOT/.env"

WORKFLOW_DIR="$REPO_ROOT/workflows/swebench-verified"
REQUIREMENTS="$WORKFLOW_DIR/requirements.txt"
ASSERT_SCRIPT="$WORKFLOW_DIR/assert_regression.py"
INSTANCE_ID="sympy__sympy-20590"
CONFIG=""
AGENT_BIN=""

if [[ ! -d "$WORKFLOW_DIR" ]]; then
  cat >&2 <<'EOF'
workflows/swebench-verified not found (gitignored copy-unit).

Restore from git history for local testing, for example:
  git checkout HEAD~1 -- workflows
  # or: git checkout <commit-before-untrack> -- workflows
EOF
  exit 1
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      CONFIG="${2:-}"
      shift 2
      ;;
    --instance-id)
      INSTANCE_ID="${2:-}"
      shift 2
      ;;
    --agent-bin)
      AGENT_BIN="${2:-}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ ! -f "$ASSERT_SCRIPT" ]]; then
  echo "assert_regression.py not found: $ASSERT_SCRIPT" >&2
  exit 1
fi

if [[ -z "$CONFIG" ]]; then
  LOCAL_CONFIG="$REPO_ROOT/agent-config.local.yaml"
  EXAMPLE_CONFIG="$REPO_ROOT/test-agents/swebench-solver/agent-config.example.yaml"
  if [[ -f "$LOCAL_CONFIG" ]]; then
    CONFIG="$LOCAL_CONFIG"
  else
    CONFIG="$EXAMPLE_CONFIG"
  fi
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "SWE-bench regression config not found: $CONFIG" >&2
  exit 1
fi

CONFIG_TEXT="$(cat "$CONFIG")"
API_KEY_ENV="$(yaml_scalar "api_key_env" "OPENAI_API_KEY" <<<"$CONFIG_TEXT")"
if [[ -z "${!API_KEY_ENV:-}" ]]; then
  cat >&2 <<EOF
SWE-bench regression requires a provider API key via environment.

Set:
  - environment variable $API_KEY_ENV
  - $API_KEY_ENV in $REPO_ROOT/.env

Inline provider.api_key in config is rejected by ConfigSafetyValidator.
EOF
  exit 1
fi

if command -v python3 >/dev/null 2>&1; then
  PYTHON=python3
elif command -v python >/dev/null 2>&1; then
  PYTHON=python
else
  echo "SWE-bench regression requires python3 on PATH." >&2
  exit 1
fi

WIN_STUBS="$WORKFLOW_DIR/win_stubs"
if [[ -d "$WIN_STUBS" ]]; then
  export PYTHONPATH="${WIN_STUBS}${PYTHONPATH:+:$PYTHONPATH}"
fi

echo "Checking Python deps (pip install -r workflows/swebench-verified/requirements.txt)..."
if ! "$PYTHON" -c "import importlib.metadata, docker, mcp; print('swebench', importlib.metadata.version('swebench'))"; then
  cat >&2 <<EOF
SWE-bench Python deps missing or broken.

Install pinned deps:
  pip install -r $REQUIREMENTS
EOF
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "SWE-bench regression requires docker CLI on PATH." >&2
  exit 1
fi

echo "Checking Docker Linux engine..."
"$PYTHON" -c "
import sys
sys.path.insert(0, r'$WORKFLOW_DIR')
from runtime import check_docker_linux
check_docker_linux()
print('Docker Linux OK')
"

RUN_ID="regression-$(date +%Y%m%d-%H%M%S)"
REPORT_PATH="$WORKFLOW_DIR/runs/$RUN_ID/report.json"
CONFIG_ABS="$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")"

echo "SWE-bench regression: instance=$INSTANCE_ID run_id=$RUN_ID config=$CONFIG_ABS"
RUN_ARGS=(
  run
  --instance-id "$INSTANCE_ID"
  --run-id "$RUN_ID"
  --workers 1
  --config "$CONFIG_ABS"
  --repo-root "$REPO_ROOT"
)

if [[ -n "$AGENT_BIN" ]]; then
  if [[ ! -f "$AGENT_BIN" ]]; then
    echo "agent binary not found: $AGENT_BIN" >&2
    exit 1
  fi
  RUN_ARGS+=(--agent-bin "$AGENT_BIN")
  echo "Using --agent-bin $AGENT_BIN (skip cargo build)"
else
  echo "Building release agent..."
  cargo build --release
fi

set +e
"$SCRIPT_DIR/swebench-verified-run.sh" "${RUN_ARGS[@]}"
RUN_EXIT=$?
set -e

if [[ "$RUN_EXIT" -ne 0 ]]; then
  echo "SWE-bench run exited with code $RUN_EXIT (still asserting report if present)."
fi

set +e
"$PYTHON" "$ASSERT_SCRIPT" "$REPORT_PATH" --instance-id "$INSTANCE_ID"
ASSERT_EXIT=$?
set -e

if [[ "$ASSERT_EXIT" -ne 0 ]]; then
  exit "$ASSERT_EXIT"
fi
if [[ "$RUN_EXIT" -ne 0 ]]; then
  exit "$RUN_EXIT"
fi

echo "SWE-bench regression passed."
