#!/usr/bin/env bash
# Run Referent (or another settings profile) against local AoC bank tasks and
# compare RunOutput.result to expected answers. Linux counterpart of aoc-eval.ps1.
#
# Task bank and run artifacts stay outside git (local/aoc-bank, local/aoc-runs).
# This script is the eval harness — it is not cargo test / CI.
#
# Usage:
#   ./scripts/aoc-eval.sh
#   ./scripts/aoc-eval.sh --task-id 2024-01-1
#   ./scripts/aoc-eval.sh --bank-dir ./local/aoc-bank --config ./agent-config.local.yaml
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AOC_LIB="$SCRIPT_DIR/aoc-lib.py"

TASK_IDS=()
BANK_DIR=""
CONFIG=""
SETTINGS_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --task-id)
      TASK_IDS+=("${2:-}")
      shift 2
      ;;
    --bank-dir)
      BANK_DIR="${2:-}"
      shift 2
      ;;
    --config)
      CONFIG="${2:-}"
      shift 2
      ;;
    --settings-dir)
      SETTINGS_DIR="${2:-}"
      shift 2
      ;;
    --repo-root)
      REPO_ROOT="$(cd "${2:-}" && pwd)"
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

cd "$REPO_ROOT"

# shellcheck source=import-dotenv.sh
source "$SCRIPT_DIR/import-dotenv.sh"
import_dotenv "$REPO_ROOT/.env"

if [[ -z "$BANK_DIR" ]]; then
  BANK_DIR="$REPO_ROOT/local/aoc-bank"
else
  BANK_DIR="$(cd "$BANK_DIR" && pwd)"
fi

if [[ -z "$CONFIG" ]]; then
  CONFIG="$REPO_ROOT/test-agents/referent/agent-config.aoc.example.yaml"
else
  CONFIG="$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")"
fi

if [[ -z "$SETTINGS_DIR" ]]; then
  SETTINGS_DIR="$REPO_ROOT/test-agents/referent"
else
  SETTINGS_DIR="$(cd "$SETTINGS_DIR" && pwd)"
fi

if [[ ! -d "$BANK_DIR" ]]; then
  echo "AoC bank not found: $BANK_DIR" >&2
  echo "Copy local/aoc-bank.example to local/aoc-bank and fill tasks." >&2
  exit 1
fi
if [[ ! -f "$CONFIG" ]]; then
  echo "Config not found: $CONFIG" >&2
  exit 1
fi
if [[ ! -d "$SETTINGS_DIR" ]]; then
  echo "Settings dir not found: $SETTINGS_DIR" >&2
  exit 1
fi

resolve_python() {
  local candidate
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  echo "Could not resolve python3/python for sandboxed home.run." >&2
  exit 1
}

json_get() {
  local blob="$1"
  local key="$2"
  python3 -c 'import json,sys; print(json.load(sys.stdin).get(sys.argv[1]) or "")' "$key" <<<"$blob"
}

json_get_raw() {
  local blob="$1"
  local key="$2"
  python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin).get(sys.argv[1])))' "$key" <<<"$blob"
}

BASE_CONFIG_TEXT="$(cat "$CONFIG")"
PROVIDER_BASE_URL="$(yaml_scalar "base_url" "https://polza.ai/api/v1" <<<"$BASE_CONFIG_TEXT")"
PROVIDER_MODEL="$(yaml_scalar "model" "openai/gpt-5.6-luna-pro" <<<"$BASE_CONFIG_TEXT")"
PROVIDER_API_KEY_ENV="$(yaml_scalar "api_key_env" "POLZA_API_KEY" <<<"$BASE_CONFIG_TEXT")"
PROVIDER_API_KEY="$(yaml_provider_api_key <<<"$BASE_CONFIG_TEXT")"
PROVIDER_TIMEOUT_MS="$(yaml_scalar "timeout_ms" "180000" <<<"$BASE_CONFIG_TEXT")"
MAX_ITERATIONS="$(yaml_scalar "max_iterations" "40" <<<"$BASE_CONFIG_TEXT")"
MAX_TOKENS="$(yaml_scalar "max_tokens" "500000" <<<"$BASE_CONFIG_TEXT")"
MAX_DURATION_SEC="$(yaml_scalar "max_duration_sec" "900" <<<"$BASE_CONFIG_TEXT")"

# On Linux, use the host interpreter directly (namespace mounts handle roots).
PYTHON_EXE="$(resolve_python)"
PYTHON_ROOT="$(cd "$(dirname "$PYTHON_EXE")" && pwd)"
PYTHON_INHERIT_ENV='[]'

echo "sandbox python=$PYTHON_EXE root=$PYTHON_ROOT"

mapfile -t TASK_FILES < <(find "$BANK_DIR" -maxdepth 1 -type f -name '*.json' | sort)
if [[ ${#TASK_FILES[@]} -eq 0 ]]; then
  echo "No JSON tasks in $BANK_DIR" >&2
  exit 1
fi

TASK_LINES=()
for file in "${TASK_FILES[@]}"; do
  meta="$(python3 "$AOC_LIB" task-meta "$file")"
  tid="${meta%%$'\t'*}"
  if [[ ${#TASK_IDS[@]} -gt 0 ]]; then
    matched=0
    for want in "${TASK_IDS[@]}"; do
      if [[ "$tid" == "$want" ]]; then
        matched=1
        break
      fi
    done
    if [[ "$matched" -eq 0 ]]; then
      continue
    fi
  fi
  TASK_LINES+=("$meta")
done

if [[ ${#TASK_LINES[@]} -eq 0 ]]; then
  echo "No tasks matched the requested --task-id filter." >&2
  exit 1
fi

RUN_ID="$(date +%Y%m%d-%H%M%S)"
RUNS_ROOT="$REPO_ROOT/local/aoc-runs/$RUN_ID"
mkdir -p "$RUNS_ROOT"

export AOC_BANK_DIR="$BANK_DIR"

PASSED=0
FAILED=0
REPORT_TASKS_JSON='[]'

echo "AoC eval run=$RUN_ID bank=$BANK_DIR tasks=${#TASK_LINES[@]}"
echo "config=$CONFIG settings=$SETTINGS_DIR model=$PROVIDER_MODEL"

AGENT_BIN="$REPO_ROOT/target/release/agent_Kuibyshev"
if [[ ! -x "$AGENT_BIN" ]]; then
  echo "Release binary missing: $AGENT_BIN (run cargo build --release first)" >&2
  exit 1
fi

escape_yaml_dq() {
  printf '%s' "$1" | sed 's/"/\\"/g'
}

for meta in "${TASK_LINES[@]}"; do
  IFS=$'\t' read -r TASK_ID EXPECTED TASK_PATH <<<"$meta"
  HOME_DIR="$RUNS_ROOT/$TASK_ID"
  LOG_DIR="$HOME_DIR/logs"
  mkdir -p "$HOME_DIR/in" "$HOME_DIR/out" "$LOG_DIR"

  export AOC_HOME_DIR="$HOME_DIR"
  export AOC_BANK_DIR="$BANK_DIR"

  RUN_CONFIG_PATH="$HOME_DIR/agent-config.yaml"
  PROVIDER_API_KEY_LINE=""
  if [[ -n "$PROVIDER_API_KEY" ]]; then
    ESCAPED_KEY="$(escape_yaml_dq "$PROVIDER_API_KEY")"
    PROVIDER_API_KEY_LINE="  api_key: \"$ESCAPED_KEY\""$'\n'
  fi

  cat >"$RUN_CONFIG_PATH" <<EOF
provider:
  base_url: "$PROVIDER_BASE_URL"
  model: "$PROVIDER_MODEL"
${PROVIDER_API_KEY_LINE}  api_key_env: "$PROVIDER_API_KEY_ENV"
  timeout_ms: $PROVIDER_TIMEOUT_MS
  max_retries: 3
  retry_base_delay_ms: 500

mcp:
  - name: "aoc"
    command: "node"
    args:
      - "./mcp-aoc-tasks.js"
      - "--bank-dir=$BANK_DIR"
      - "--home-dir=$HOME_DIR"
    env:
      AOC_BANK_DIR: "$BANK_DIR"
      AOC_HOME_DIR: "$HOME_DIR"
    timeout_ms: 30000

limits:
  max_iterations: $MAX_ITERATIONS
  max_tokens: $MAX_TOKENS
  max_duration_sec: $MAX_DURATION_SEC

logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: true
  output_dir: "$LOG_DIR"

# Fail-closed OS sandbox for home.run (Linux namespaces).
access:
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
      - home.run
  filesystem:
    home:
      # AoC solutions and input.txt live at home root; in/out kept for artifacts.
      read: [".", "in", "out"]
      write: [".", "out"]
  run:
    programs:
      - name: python
        executable: "$PYTHON_EXE"
        runtime_read_roots: ["$PYTHON_ROOT"]
        inherit_env: $PYTHON_INHERIT_ENV
        allow_children: false
    max_args: 32
    max_arg_chars: 4096
    max_output_chars: 200000
    max_timeout_ms: 120000
EOF

  python3 "$AOC_LIB" seed-input "$TASK_PATH" "$HOME_DIR/input.txt"

  PROMPT="Solve AoC task ${TASK_ID}. Work one turn at a time: each reply must be exactly one JSON object (never multiple JSON objects). Do not pre-emit future turns. Steps across turns: 1) Fetch statement with aoc_get_task and call aoc_get_input (writes/confirm home/input.txt; do not paste the full input into thoughts). input.txt is already present under home. 2) Write solution.py that reads input.txt, then home.run with program=python. Debug until stdout shows the correct answer. 3) Final response: done=true with result equal to only the final answer string. Do not guess. Return JSON only on every turn."

  echo ""
  echo "=== $TASK_ID ==="

  STDOUT_PATH="$HOME_DIR/agent.stdout.json"
  STDERR_PATH="$HOME_DIR/agent.stderr.txt"

  ENTRY_PASS=false
  ENTRY_STOP=""
  ENTRY_RESULT=""
  ENTRY_USAGE="null"
  ENTRY_ERROR=""
  ENTRY_LOGS="null"
  ENTRY_ELAPSED=0

  START_NS="$(date +%s%N)"
  set +e
  "$AGENT_BIN" \
    --config "$RUN_CONFIG_PATH" \
    --settings-dir "$SETTINGS_DIR" \
    --prompt "$PROMPT" \
    --home "$HOME_DIR" \
    --save-chat-history \
    >"$STDOUT_PATH" 2>"$STDERR_PATH"
  EXIT_CODE=$?
  set -e
  END_NS="$(date +%s%N)"
  ENTRY_ELAPSED=$(( (END_NS - START_NS) / 1000000 ))

  if [[ "$EXIT_CODE" -ne 0 ]]; then
    ENTRY_ERROR="agent exited with code ${EXIT_CODE}: $(cat "$STDERR_PATH")"
    FAILED=$((FAILED + 1))
    echo "FAIL $TASK_ID: $ENTRY_ERROR"
  else
    set +e
    PARSE_OUT="$(python3 "$AOC_LIB" parse-stdout "$STDOUT_PATH" 2>/tmp/aoc-eval-parse.err)"
    PARSE_RC=$?
    set -e
    if [[ "$PARSE_RC" -ne 0 || -z "$PARSE_OUT" ]]; then
      ENTRY_ERROR="failed to parse agent stdout: $(cat /tmp/aoc-eval-parse.err 2>/dev/null || true)"
      FAILED=$((FAILED + 1))
      echo "FAIL $TASK_ID: $ENTRY_ERROR"
    else
      ENTRY_RESULT="$(json_get "$PARSE_OUT" result)"
      ENTRY_STOP="$(json_get "$PARSE_OUT" stop_reason)"
      ENTRY_USAGE="$(json_get_raw "$PARSE_OUT" usage)"
      ENTRY_LOGS="$(json_get_raw "$PARSE_OUT" logs)"

      if [[ "$ENTRY_STOP" == "goal_reached" && "$ENTRY_RESULT" == "$EXPECTED" ]]; then
        ENTRY_PASS=true
        PASSED=$((PASSED + 1))
        echo "PASS $TASK_ID result=$ENTRY_RESULT"
        echo "Logs dir: $LOG_DIR"
        python3 "$AOC_LIB" print-logs "$ENTRY_LOGS"
      else
        FAILED=$((FAILED + 1))
        echo "FAIL $TASK_ID stop=$ENTRY_STOP result='$ENTRY_RESULT' expected='$EXPECTED'"
      fi
    fi
  fi

  REPORT_TASKS_JSON="$(python3 "$AOC_LIB" append-task \
    "$REPORT_TASKS_JSON" \
    "$TASK_ID" \
    "$EXPECTED" \
    "$ENTRY_PASS" \
    "$ENTRY_STOP" \
    "$ENTRY_RESULT" \
    "$ENTRY_USAGE" \
    "$ENTRY_ERROR" \
    "$HOME_DIR" \
    "$LOG_DIR" \
    "$ENTRY_LOGS" \
    "$ENTRY_ELAPSED")"
done

REPORT_PATH="$RUNS_ROOT/report.json"
python3 "$AOC_LIB" write-report \
  "$RUN_ID" \
  "$BANK_DIR" \
  "$CONFIG" \
  "$SETTINGS_DIR" \
  "$PASSED" \
  "$FAILED" \
  "${#TASK_LINES[@]}" \
  "$REPORT_TASKS_JSON" \
  "$REPORT_PATH"

echo ""
echo "Report: $REPORT_PATH"
echo "Summary: passed=$PASSED failed=$FAILED total=${#TASK_LINES[@]}"

if [[ "$FAILED" -gt 0 ]]; then
  exit 1
fi
exit 0
