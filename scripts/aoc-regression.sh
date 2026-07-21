#!/usr/bin/env bash
# Prerequisites check + AoC single-agent regression eval (Linux).
#
# Fails if the local bank, config, or API key are missing. Intended to run from
# scripts/check.sh on every local quality gate.
#
# Usage:
#   ./scripts/aoc-regression.sh
#   ./scripts/aoc-regression.sh --config path/to/config.yaml
#   ./scripts/aoc-regression.sh --task-id 2024-01-1
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# shellcheck source=import-dotenv.sh
source "$SCRIPT_DIR/import-dotenv.sh"
import_dotenv "$REPO_ROOT/.env"

CONFIG=""
TASK_IDS=()

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
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

BANK_DIR="$REPO_ROOT/local/aoc-bank"
if [[ ! -d "$BANK_DIR" ]]; then
  cat >&2 <<EOF
AoC regression bank not found: $BANK_DIR

Copy the example and fill real tasks (gitignored):
  cp -R ./local/aoc-bank.example ./local/aoc-bank
EOF
  exit 1
fi

TASK_COUNT="$(find "$BANK_DIR" -maxdepth 1 -type f -name '*.json' | wc -l | tr -d ' ')"
if [[ "$TASK_COUNT" -eq 0 ]]; then
  echo "AoC regression bank is empty: $BANK_DIR" >&2
  exit 1
fi

if [[ -z "$CONFIG" ]]; then
  LOCAL_CONFIG="$REPO_ROOT/agent-config.local.yaml"
  EXAMPLE_CONFIG="$REPO_ROOT/test-agents/referent/agent-config.aoc.example.yaml"
  if [[ -f "$LOCAL_CONFIG" ]]; then
    CONFIG="$LOCAL_CONFIG"
  else
    CONFIG="$EXAMPLE_CONFIG"
  fi
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "AoC regression config not found: $CONFIG" >&2
  exit 1
fi

CONFIG_TEXT="$(cat "$CONFIG")"
if ! provider_api_key_available "$CONFIG_TEXT"; then
  API_KEY_ENV="$(yaml_scalar "api_key_env" "OPENAI_API_KEY" <<<"$CONFIG_TEXT")"
  cat >&2 <<EOF
AoC regression requires a provider API key.

Set one of:
  - provider.api_key in $CONFIG
  - environment variable $API_KEY_ENV
  - $API_KEY_ENV in $REPO_ROOT/.env
EOF
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "AoC regression requires Node.js on PATH (mcp-aoc-tasks.js)." >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1; then
  echo "AoC regression requires python3 on PATH (home.run solutions)." >&2
  exit 1
fi

echo "AoC regression: bank=$BANK_DIR tasks=$TASK_COUNT config=$CONFIG"
echo "Building release agent (sandboxed home.run)..."
cargo build --release

EVAL_ARGS=(--config "$CONFIG" --bank-dir "$BANK_DIR")
for tid in "${TASK_IDS[@]+"${TASK_IDS[@]}"}"; do
  EVAL_ARGS+=(--task-id "$tid")
done

"$SCRIPT_DIR/aoc-eval.sh" "${EVAL_ARGS[@]}"

echo "AoC regression passed."
