#!/usr/bin/env bash
# Live A2A regression gate (Agent Card, Bearer, SendMessage + LLM).
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
      sed -n '2,8p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

BANK_EXAMPLE="$REPO_ROOT/local/a2a-bank.example"
BANK_DIR="$REPO_ROOT/local/a2a-bank"
if [[ ! -d "$BANK_DIR" ]]; then
  if [[ -d "$BANK_EXAMPLE" ]]; then
    echo "Copying A2A bank example -> local/a2a-bank"
    cp -R "$BANK_EXAMPLE" "$BANK_DIR"
  else
    echo "A2A bank not found: $BANK_DIR" >&2
    exit 1
  fi
fi

TASK_COUNT="$(find "$BANK_DIR" -maxdepth 1 -type f -name '*.json' | wc -l | tr -d ' ')"
if [[ "$TASK_COUNT" -eq 0 ]]; then
  echo "A2A bank is empty: $BANK_DIR" >&2
  exit 1
fi

if [[ -z "$CONFIG" ]]; then
  LOCAL_CONFIG="$REPO_ROOT/agent-config.local.yaml"
  EXAMPLE_CONFIG="$REPO_ROOT/test-agents/a2a-probe/agent-config.example.yaml"
  if [[ -f "$LOCAL_CONFIG" ]]; then
    CONFIG="$LOCAL_CONFIG"
  else
    CONFIG="$EXAMPLE_CONFIG"
  fi
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "A2A regression config not found: $CONFIG" >&2
  exit 1
fi

CONFIG_TEXT="$(cat "$CONFIG")"
if ! python3 "$SCRIPT_DIR/aoc-lib.py" api_key_available <<<"$CONFIG_TEXT" 2>/dev/null; then
  HAS_SEND=0
  for file in "$BANK_DIR"/*.json; do
    [[ -f "$file" ]] || continue
    kind="$(python3 -c "import json,sys; o=json.load(open(sys.argv[1], encoding='utf-8')); print(o.get('kind','send'))" "$file")"
    tid="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8'))['id'])" "$file")"
    if [[ "$kind" == "send" ]]; then
      if [[ ${#TASK_IDS[@]} -eq 0 ]]; then
        HAS_SEND=1
        break
      fi
      for want in "${TASK_IDS[@]}"; do
        if [[ "$want" == "$tid" ]]; then
          HAS_SEND=1
          break 2
        fi
      done
    fi
  done
  if [[ "$HAS_SEND" -eq 1 ]]; then
    API_KEY_ENV="$(python3 "$SCRIPT_DIR/aoc-lib.py" yaml_scalar api_key_env OPENAI_API_KEY <<<"$CONFIG_TEXT")"
    cat >&2 <<EOF
A2A send tasks require a provider API key via environment.

Set:
  - environment variable $API_KEY_ENV
  - $API_KEY_ENV in $REPO_ROOT/.env
EOF
    exit 1
  fi
fi

echo "Building release agent..."
cargo build --release -p agent_Kuibysheff --bin kbshff

PY_ARGS=(
  "$SCRIPT_DIR/a2a-eval.py"
  --repo-root "$REPO_ROOT"
  --bank-dir "$BANK_DIR"
  --config "$CONFIG"
)
for tid in "${TASK_IDS[@]+"${TASK_IDS[@]}"}"; do
  PY_ARGS+=(--task-id "$tid")
done

echo "A2A regression: bank=$BANK_DIR tasks=$TASK_COUNT config=$CONFIG"
python3 "${PY_ARGS[@]}"
echo "A2A regression passed."
