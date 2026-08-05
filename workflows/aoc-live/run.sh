#!/usr/bin/env bash
# Launch the live AoC ACP singleton workflow example.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

YEAR=""
DAY=""
PART=1
MAX_ATTEMPTS=5
CONFIG=""
SETTINGS_DIR=""
AGENT_BIN=""
VERBOSE=""

usage() {
  cat <<EOF
Usage: $0 --year YYYY --day N [--part 1|2] [--max-attempts N] [options]

Options:
  --year YYYY          Advent of Code year (required)
  --day N              Puzzle day 1..25 (required)
  --part N             Part 1 or 2 (default: 1)
  --max-attempts N     Full solve/submit iterations (default: 5, cap 5)
  --config PATH        Base agent config template
  --settings-dir PATH  Agent settings directory
  --agent-bin PATH     Path to agent_Kuibyshev binary
  --repo-root PATH     Repository root (default: auto)
  -v, --verbose        Debug logging
  -h, --help           Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --year) YEAR="$2"; shift 2 ;;
    --day) DAY="$2"; shift 2 ;;
    --part) PART="$2"; shift 2 ;;
    --max-attempts) MAX_ATTEMPTS="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --settings-dir) SETTINGS_DIR="$2"; shift 2 ;;
    --agent-bin) AGENT_BIN="$2"; shift 2 ;;
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

if [[ -f "${REPO_ROOT}/scripts/import-dotenv.sh" ]]; then
  # shellcheck disable=SC1091
  source "${REPO_ROOT}/scripts/import-dotenv.sh"
  import_dotenv "${REPO_ROOT}/.env" || true
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
  --repo-root "$REPO_ROOT"
)
[[ -n "$CONFIG" ]] && ARGS+=(--config "$CONFIG")
[[ -n "$SETTINGS_DIR" ]] && ARGS+=(--settings-dir "$SETTINGS_DIR")
[[ -n "$AGENT_BIN" ]] && ARGS+=(--agent-bin "$AGENT_BIN")
[[ -n "$VERBOSE" ]] && ARGS+=("$VERBOSE")

echo "Running: ${PYTHON_BIN} ${ARGS[*]}"
exec "$PYTHON_BIN" "${ARGS[@]}"
