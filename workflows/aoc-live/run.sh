#!/usr/bin/env bash
# Launch the live AoC ACP singleton workflow example.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT=""

YEAR=""
DAY=""
PART=1
MAX_ATTEMPTS=5
PROJECT_ROOT=""
AGENT=""
HOME_REL=""
IMPORT_FROM=""
CONFIG=""
SETTINGS_DIR=""
AGENT_BIN=""
MCP_JS=""
VERBOSE=""

usage() {
  cat <<EOF
Usage: $0 --year YYYY --day N [--part 1|2] [--max-attempts N] [options]

Options:
  --year YYYY            Advent of Code year (required)
  --day N                Puzzle day 1..25 (required)
  --part N               Part 1 or 2 (default: 1)
  --max-attempts N       Full solve/submit iterations (default: 5, cap 5)
  --project-root PATH    Project owning .kuibysheff/ (default: local/aoc-live-project)
  --agent ID             Agent id (default: aoc-live)
  --home REL             Relative home under .kuibysheff/ (default: homes/<run-id>)
  --import-from PATH     Template dir imported into protected profile
  --config PATH          Provider config template (import/render source only)
  --settings-dir PATH    Legacy alias for --import-from
  --agent-bin PATH       Path to agent_Kuibysheff binary
  --mcp-js PATH          Path to mcp-aoc-tasks.js
  --repo-root PATH       Optional monorepo root (Cargo / staged Python)
  -v, --verbose          Debug logging
  -h, --help             Show help
EOF
}

import_dotenv() {
  local path="$1"
  [[ -f "$path" ]] || return 0
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    [[ -z "$line" || "$line" == \#* || "$line" != *=* ]] && continue
    local key="${line%%=*}"
    local value="${line#*=}"
    key="$(echo "$key" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    value="$(echo "$value" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//;s/^["'\'']//;s/["'\'']$//')"
    [[ -z "$key" ]] && continue
    if [[ -n "${!key+x}" ]]; then
      continue
    fi
    export "$key=$value"
  done < "$path"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --year) YEAR="$2"; shift 2 ;;
    --day) DAY="$2"; shift 2 ;;
    --part) PART="$2"; shift 2 ;;
    --max-attempts) MAX_ATTEMPTS="$2"; shift 2 ;;
    --project-root) PROJECT_ROOT="$2"; shift 2 ;;
    --agent) AGENT="$2"; shift 2 ;;
    --home) HOME_REL="$2"; shift 2 ;;
    --import-from) IMPORT_FROM="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --settings-dir) SETTINGS_DIR="$2"; shift 2 ;;
    --agent-bin) AGENT_BIN="$2"; shift 2 ;;
    --mcp-js) MCP_JS="$2"; shift 2 ;;
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    -v|--verbose) VERBOSE="-v"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "$YEAR" || -z "$DAY" ]]; then
  usage
  exit 2
fi

import_dotenv "${SCRIPT_DIR}/.env"
import_dotenv "$(pwd)/.env"
if [[ -n "$REPO_ROOT" ]]; then
  import_dotenv "${REPO_ROOT}/.env"
elif [[ -f "${SCRIPT_DIR}/../../.env" ]]; then
  import_dotenv "${SCRIPT_DIR}/../../.env"
fi
if [[ -z "$REPO_ROOT" && -f "${SCRIPT_DIR}/../../Cargo.toml" ]]; then
  REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
fi

if [[ -z "${AOC_SESSION:-}" ]]; then
  echo "AOC_SESSION is not set. Put your AoC session cookie in the environment or .env." >&2
  exit 1
fi

PYTHON_BIN="$(command -v python3 || command -v python || true)"
if [[ -z "$PYTHON_BIN" ]]; then
  echo "python3/python not found on PATH" >&2
  exit 1
fi

ARGS=(
  "${SCRIPT_DIR}/aoc-singleton.py"
  --year "$YEAR"
  --day "$DAY"
  --part "$PART"
  --max-attempts "$MAX_ATTEMPTS"
)
[[ -n "$REPO_ROOT" ]] && ARGS+=(--repo-root "$REPO_ROOT")
[[ -n "$PROJECT_ROOT" ]] && ARGS+=(--project-root "$PROJECT_ROOT")
[[ -n "$AGENT" ]] && ARGS+=(--agent "$AGENT")
[[ -n "$HOME_REL" ]] && ARGS+=(--home "$HOME_REL")
if [[ -n "$IMPORT_FROM" ]]; then
  ARGS+=(--import-from "$IMPORT_FROM")
elif [[ -n "$SETTINGS_DIR" ]]; then
  ARGS+=(--settings-dir "$SETTINGS_DIR")
fi
[[ -n "$CONFIG" ]] && ARGS+=(--config "$CONFIG")
[[ -n "$AGENT_BIN" ]] && ARGS+=(--agent-bin "$AGENT_BIN")
[[ -n "$MCP_JS" ]] && ARGS+=(--mcp-js "$MCP_JS")
[[ -n "$VERBOSE" ]] && ARGS+=("$VERBOSE")

echo "Running: ${PYTHON_BIN} ${ARGS[*]}"
exec "$PYTHON_BIN" "${ARGS[@]}"
