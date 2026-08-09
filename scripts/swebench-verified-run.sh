#!/usr/bin/env bash
# Thin forwarder to workflows/swebench-verified/run.sh (monorepo UX).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHER="${SCRIPT_DIR}/../workflows/swebench-verified/run.sh"

if [[ ! -f "$LAUNCHER" ]]; then
  echo "Workflow launcher not found: $LAUNCHER" >&2
  exit 1
fi

exec bash "$LAUNCHER" "$@"
