#!/usr/bin/env bash
# Thin wrapper: run SWE-bench regression from sibling / KUIBYSHEFF_SWEBENCH_ROOT.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARENT="$(cd "$AGENT_ROOT/.." && pwd)"

SWE_ROOT="${KUIBYSHEFF_SWEBENCH_ROOT:-}"
if [[ -z "$SWE_ROOT" || ! -f "$SWE_ROOT/scripts/swebench-regression.sh" ]]; then
  if [[ -f "$PARENT/kuibysheff-swebench/scripts/swebench-regression.sh" ]]; then
    SWE_ROOT="$PARENT/kuibysheff-swebench"
  fi
fi

if [[ -z "$SWE_ROOT" || ! -f "$SWE_ROOT/scripts/swebench-regression.sh" ]]; then
  cat >&2 <<'EOF'
SWE-bench example repo not found.

Clone https://github.com/gazalievtimur/kuibysheff-swebench next to this repo, or set:
  KUIBYSHEFF_SWEBENCH_ROOT=/path/to/kuibysheff-swebench

Then re-run: ./scripts/check.sh --swebench
EOF
  exit 1
fi

export KUIBYSHEFF_SRC="$AGENT_ROOT"
echo "Delegating SWE-bench regression to $SWE_ROOT (KUIBYSHEFF_SRC=$KUIBYSHEFF_SRC)"
exec "$SWE_ROOT/scripts/swebench-regression.sh" "$@"
