#!/usr/bin/env bash
# Live Scale-FS LLM regression gate (Linux).
#
# Usage:
#   ./scripts/scale-fs-regression.sh
#   ./scripts/scale-fs-regression.sh --config path/to/config.yaml
#   ./scripts/scale-fs-regression.sh --task-id search-many-01
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# shellcheck source=import-dotenv.sh
source "$SCRIPT_DIR/import-dotenv.sh"
import_dotenv "$REPO_ROOT/.env"

CONFIG=""
TASK_IDS=()
WORKFLOW_DIR="$REPO_ROOT/workflows/scale-fs-live"

if [[ ! -d "$WORKFLOW_DIR" ]]; then
  cat >&2 <<'EOF'
workflows/scale-fs-live not found (gitignored copy-unit).

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
    --task-id)
      TASK_IDS+=("${2:-}")
      shift 2
      ;;
    -h|--help)
      sed -n '2,10p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

BANK_EXAMPLE="$REPO_ROOT/local/scale-fs-bank.example"
BANK_DIR="$REPO_ROOT/local/scale-fs-bank"
if [[ ! -d "$BANK_DIR" ]]; then
  if [[ -d "$BANK_EXAMPLE" ]]; then
    echo "Copying scale-fs bank example -> local/scale-fs-bank"
    cp -R "$BANK_EXAMPLE" "$BANK_DIR"
  else
    echo "Scale-FS bank not found: $BANK_DIR" >&2
    exit 1
  fi
fi

TASK_COUNT="$(find "$BANK_DIR" -maxdepth 1 -type f -name '*.json' | wc -l | tr -d ' ')"
if [[ "$TASK_COUNT" -eq 0 ]]; then
  echo "Scale-FS bank is empty: $BANK_DIR" >&2
  exit 1
fi

if [[ -z "$CONFIG" ]]; then
  LOCAL_CONFIG="$REPO_ROOT/agent-config.local.yaml"
  EXAMPLE_CONFIG="$REPO_ROOT/test-agents/scale-fs-probe/agent-config.example.yaml"
  if [[ -f "$LOCAL_CONFIG" ]]; then
    CONFIG="$LOCAL_CONFIG"
  else
    CONFIG="$EXAMPLE_CONFIG"
  fi
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "Scale-FS regression config not found: $CONFIG" >&2
  exit 1
fi

CONFIG_TEXT="$(cat "$CONFIG")"
API_KEY_ENV="$(yaml_scalar "api_key_env" "OPENAI_API_KEY" <<<"$CONFIG_TEXT")"
if [[ -z "${!API_KEY_ENV:-}" ]]; then
  cat >&2 <<EOF
Scale-FS regression requires a provider API key via environment.

Set:
  - environment variable $API_KEY_ENV
  - $API_KEY_ENV in $REPO_ROOT/.env
EOF
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1; then
  echo "Scale-FS regression requires python3 on PATH." >&2
  exit 1
fi
PYTHON=python3
command -v python3 >/dev/null 2>&1 || PYTHON=python

echo "Building release agent..."
cargo build --release -p agent_Kuibysheff --bin kbshff

SETTINGS_DIR="$REPO_ROOT/test-agents/scale-fs-probe"
RUNS_ROOT="$REPO_ROOT/local/scale-fs-runs"
mkdir -p "$RUNS_ROOT"

PY_ARGS=(
  "$REPO_ROOT/workflows/scale-fs-live/eval.py"
  --repo-root "$REPO_ROOT"
  --bank-dir "$BANK_DIR"
  --config "$CONFIG"
  --settings-dir "$SETTINGS_DIR"
  --runs-root "$RUNS_ROOT"
)
for id in "${TASK_IDS[@]:-}"; do
  [[ -n "$id" ]] && PY_ARGS+=(--task-id "$id")
done

echo "Scale-FS regression: bank=$BANK_DIR tasks=$TASK_COUNT config=$CONFIG"
"$PYTHON" "${PY_ARGS[@]}"

LATEST_PTR="$RUNS_ROOT/LATEST"
if [[ ! -f "$LATEST_PTR" ]]; then
  echo "Scale-FS regression: LATEST pointer missing under local/scale-fs-runs" >&2
  exit 1
fi
RUN_DIR="$(tr -d '\r\n' < "$LATEST_PTR")"
REPORT="$RUN_DIR/report.json"
if [[ ! -f "$REPORT" ]]; then
  echo "Scale-FS regression: report.json not found: $REPORT" >&2
  exit 1
fi

"$PYTHON" "$REPO_ROOT/workflows/scale-fs-live/assert_regression.py" "$REPORT"
echo "Scale-FS regression passed."
