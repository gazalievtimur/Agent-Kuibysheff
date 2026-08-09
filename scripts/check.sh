#!/usr/bin/env bash
# Local quality gate (Linux counterpart of check.ps1).
#
# Usage:
#   ./scripts/check.sh
#   ./scripts/check.sh --skip-aoc
#   ./scripts/check.sh --swebench
#   ./scripts/check.sh --skip-deny
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKIP_AOC=0
RUN_SWEBENCH_FLAG=0
SKIP_DENY_FLAG=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-aoc|-SkipAoc)
      SKIP_AOC=1
      shift
      ;;
    --swebench|-Swebench)
      RUN_SWEBENCH_FLAG=1
      shift
      ;;
    --skip-deny|-SkipDeny)
      SKIP_DENY_FLAG=1
      shift
      ;;
    -h|--help)
      sed -n '2,10p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

require_cargo_deny() {
  if ! cargo deny --version >/dev/null 2>&1; then
    echo "cargo-deny is not installed. Install with: cargo install --locked cargo-deny" >&2
    exit 1
  fi
}

echo "Checking formatting..."
cargo fmt --all -- --check

echo "Running clippy..."
cargo clippy --workspace --all-targets -- -D warnings

if [[ "$SKIP_DENY_FLAG" -eq 1 || "${SKIP_DENY:-}" == "1" ]]; then
  echo "Skipping cargo deny (--skip-deny / SKIP_DENY=1)."
else
  echo "Running cargo deny..."
  require_cargo_deny
  cargo deny check
fi

echo "Running tests..."
cargo test --workspace

if [[ "$SKIP_AOC" -eq 1 ]]; then
  echo "Skipping AoC agent regression (--skip-aoc)."
else
  echo "Running AoC agent regression..."
  "$SCRIPT_DIR/aoc-regression.sh"
fi

if [[ "$RUN_SWEBENCH_FLAG" -eq 1 || "${RUN_SWEBENCH:-}" == "1" ]]; then
  echo "Running SWE-bench agent regression (--swebench / RUN_SWEBENCH=1)..."
  "$SCRIPT_DIR/swebench-regression.sh"
else
  echo "Skipping live SWE-bench regression (pass --swebench or set RUN_SWEBENCH=1)."
fi

echo "All checks passed."
