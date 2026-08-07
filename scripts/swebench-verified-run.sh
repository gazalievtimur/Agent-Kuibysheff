#!/usr/bin/env bash
# Launch the SWE-bench Verified workflow (preflight|generate|grade|report|run).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

usage() {
  cat <<EOF
Usage: $0 <preflight|generate|grade|report|run> [options...]

Forwards remaining arguments to workflows/swebench-verified/swebench.py.
EOF
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

# Optional --repo-root before remaining workflow args
EXTRA=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="$2"
      shift 2
      ;;
    *)
      EXTRA+=("$1")
      shift
      ;;
  esac
done

if [[ -f "${REPO_ROOT}/scripts/import-dotenv.sh" ]]; then
  # shellcheck disable=SC1091
  source "${REPO_ROOT}/scripts/import-dotenv.sh"
  import_dotenv "${REPO_ROOT}/.env" || true
fi

PYTHON_BIN="$(command -v python3 || command -v python || true)"
if [[ -z "$PYTHON_BIN" ]]; then
  echo "python3/python not found on PATH" >&2
  exit 1
fi

SCRIPT="${REPO_ROOT}/workflows/swebench-verified/swebench.py"
if [[ ! -f "$SCRIPT" ]]; then
  echo "Workflow entry not found: $SCRIPT" >&2
  exit 1
fi

ARGS=("$SCRIPT" "$COMMAND" --repo-root "$REPO_ROOT")
ARGS+=("${EXTRA[@]+"${EXTRA[@]}"}")

echo "Running: ${PYTHON_BIN} ${ARGS[*]}"
exec "$PYTHON_BIN" "${ARGS[@]}"
