#!/usr/bin/env bash
# Thin wrapper: run AoC regression from sibling / KUIBYSHEFF_AOC_ROOT example repo.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARENT="$(cd "$AGENT_ROOT/.." && pwd)"

AOC_ROOT="${KUIBYSHEFF_AOC_ROOT:-}"
if [[ -z "$AOC_ROOT" || ! -f "$AOC_ROOT/scripts/aoc-regression.sh" ]]; then
  if [[ -f "$PARENT/kuibysheff-aoc/scripts/aoc-regression.sh" ]]; then
    AOC_ROOT="$PARENT/kuibysheff-aoc"
  fi
fi

if [[ -z "$AOC_ROOT" || ! -f "$AOC_ROOT/scripts/aoc-regression.sh" ]]; then
  cat >&2 <<'EOF'
AoC example repo not found.

Clone https://github.com/gazalievtimur/kuibysheff-aoc next to this repo, or set:
  KUIBYSHEFF_AOC_ROOT=/path/to/kuibysheff-aoc

Then re-run: ./scripts/check.sh --aoc
EOF
  exit 1
fi

export KUIBYSHEFF_SRC="$AGENT_ROOT"
echo "Delegating AoC regression to $AOC_ROOT (KUIBYSHEFF_SRC=$KUIBYSHEFF_SRC)"
exec "$AOC_ROOT/scripts/aoc-regression.sh" "$@"
