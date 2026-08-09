#!/usr/bin/env bash
# Launch the SWE-bench Verified workflow from the copy unit folder.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT=""
AGENT_BIN=""

usage() {
  cat <<EOF
Usage: $0 <preflight|generate|grade|report|run> [options...]

Runs swebench.py from this workflow copy unit.
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

if [[ $# -lt 1 ]]; then
  usage
  exit 2
fi

COMMAND="$1"
shift

case "$COMMAND" in
  preflight|generate|grade|report|run) ;;
  -h|--help) usage; exit 0 ;;
  *) echo "Unknown command: $COMMAND" >&2; usage; exit 2 ;;
esac

EXTRA=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    --agent-bin) AGENT_BIN="$2"; shift 2 ;;
    *) EXTRA+=("$1"); shift ;;
  esac
done

import_dotenv "${SCRIPT_DIR}/.env"
import_dotenv "$(pwd)/.env"
EXPLICIT_REPO_ROOT="$REPO_ROOT"
if [[ -n "$REPO_ROOT" ]]; then
  import_dotenv "${REPO_ROOT}/.env"
elif [[ -f "${SCRIPT_DIR}/../../Cargo.toml" ]]; then
  DOTENV_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
  import_dotenv "${DOTENV_ROOT}/.env"
fi

PYTHON_BIN="$(command -v python3 || command -v python || true)"
if [[ -z "$PYTHON_BIN" ]]; then
  echo "python3/python not found on PATH" >&2
  exit 1
fi

if [[ -d "${SCRIPT_DIR}/win_stubs" ]]; then
  export PYTHONPATH="${SCRIPT_DIR}/win_stubs${PYTHONPATH:+:$PYTHONPATH}"
fi

ARGS=("${SCRIPT_DIR}/swebench.py" "$COMMAND")
[[ -n "$EXPLICIT_REPO_ROOT" ]] && ARGS+=(--repo-root "$EXPLICIT_REPO_ROOT")
[[ -n "$AGENT_BIN" ]] && ARGS+=(--agent-bin "$AGENT_BIN")
ARGS+=("${EXTRA[@]+"${EXTRA[@]}"}")

echo "Running: ${PYTHON_BIN} ${ARGS[*]}"
exec "$PYTHON_BIN" "${ARGS[@]}"
