#!/usr/bin/env bash
# Local quality gate (Linux counterpart of check.ps1).
#
# Usage:
#   ./scripts/check.sh
#   ./scripts/check.sh --skip-aoc
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKIP_AOC=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-aoc|-SkipAoc)
      SKIP_AOC=1
      shift
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

echo "Checking formatting..."
cargo fmt --all -- --check

echo "Running clippy..."
cargo clippy --all-targets -- -D warnings

echo "Running tests..."
cargo test

if [[ "$SKIP_AOC" -eq 1 ]]; then
  echo "Skipping AoC agent regression (--skip-aoc)."
else
  echo "Running AoC agent regression..."
  "$SCRIPT_DIR/aoc-regression.sh"
fi

echo "All checks passed."
