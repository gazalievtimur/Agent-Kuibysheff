#!/usr/bin/env bash
# Local quality gate (Linux counterpart of check.ps1).
#
# Usage:
#   ./scripts/check.sh
#   ./scripts/check.sh --aoc
#   ./scripts/check.sh --swebench
#   ./scripts/check.sh --security
#   ./scripts/check.sh --scale-fs
#   ./scripts/check.sh --skip-deny
#   ./scripts/check.sh --skip-aoc   # deprecated no-op (AoC is opt-in)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_AOC_FLAG=0
RUN_SWEBENCH_FLAG=0
RUN_SECURITY_FLAG=0
RUN_SCALE_FS_FLAG=0
SKIP_DENY_FLAG=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --aoc|-Aoc)
      RUN_AOC_FLAG=1
      shift
      ;;
    --skip-aoc|-SkipAoc)
      # Deprecated: offline is the default; kept for older scripts.
      shift
      ;;
    --swebench|-Swebench)
      RUN_SWEBENCH_FLAG=1
      shift
      ;;
    --security|-Security)
      RUN_SECURITY_FLAG=1
      shift
      ;;
    --scale-fs|-ScaleFs)
      RUN_SCALE_FS_FLAG=1
      shift
      ;;
    --skip-deny|-SkipDeny)
      SKIP_DENY_FLAG=1
      shift
      ;;
    -h|--help)
      sed -n '2,13p' "$0"
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

echo "Running portability guardrails..."
chmod +x "$SCRIPT_DIR/check-portability.sh"
"$SCRIPT_DIR/check-portability.sh"
if [[ -d "$SCRIPT_DIR/../workflows" ]]; then
  python3 "$SCRIPT_DIR/test_detached_workflows.py"
else
  echo "Skipping detached workflow tests (workflows/ not present)."
fi

if [[ "$RUN_AOC_FLAG" -eq 1 || "${RUN_AOC:-}" == "1" ]]; then
  echo "Running AoC agent regression (--aoc / RUN_AOC=1)..."
  "$SCRIPT_DIR/aoc-regression.sh"
else
  echo "Skipping live AoC regression (pass --aoc or set RUN_AOC=1)."
fi

if [[ "$RUN_SWEBENCH_FLAG" -eq 1 || "${RUN_SWEBENCH:-}" == "1" ]]; then
  echo "Running SWE-bench agent regression (--swebench / RUN_SWEBENCH=1)..."
  "$SCRIPT_DIR/swebench-regression.sh"
else
  echo "Skipping live SWE-bench regression (pass --swebench or set RUN_SWEBENCH=1)."
fi

if [[ "$RUN_SECURITY_FLAG" -eq 1 || "${RUN_SECURITY:-}" == "1" ]]; then
  echo "Running security sandbox LLM regression (--security / RUN_SECURITY=1)..."
  "$SCRIPT_DIR/security-regression.sh"
else
  echo "Skipping live security sandbox regression (pass --security or set RUN_SECURITY=1)."
fi

if [[ "$RUN_SCALE_FS_FLAG" -eq 1 || "${RUN_SCALE_FS:-}" == "1" ]]; then
  echo "Running Scale-FS live LLM regression (--scale-fs / RUN_SCALE_FS=1)..."
  "$SCRIPT_DIR/scale-fs-regression.sh"
else
  echo "Skipping live Scale-FS regression (pass --scale-fs or set RUN_SCALE_FS=1)."
fi

echo "All checks passed."
