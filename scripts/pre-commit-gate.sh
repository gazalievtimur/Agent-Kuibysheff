#!/usr/bin/env bash
# CI-parity gate used by git pre-commit / Cursor hooks.
#
# Mirrors the fast CI jobs:
#   - cargo fmt --all -- --check
#   - cargo clippy --workspace --all-targets -- -D warnings
#   - cargo test --workspace
#   - cargo +nightly miri test -p sandbox-linux (Linux; skip with --skip-miri)
#
# Bypass: SKIP_PRECOMMIT=1
set -euo pipefail

SKIP_MIRI_FLAG=0
SKIP_TESTS=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-miri|-SkipMiri)
      SKIP_MIRI_FLAG=1
      shift
      ;;
    --skip-tests|-SkipTests)
      SKIP_TESTS=1
      shift
      ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "${SKIP_PRECOMMIT:-}" == "1" ]]; then
  echo "SKIP_PRECOMMIT=1 — bypassing pre-commit gate."
  exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() {
  local label="$1"
  shift
  echo ""
  echo "==> $label"
  "$@"
}

step "cargo fmt --all -- --check" \
  cargo fmt --all -- --check

step "cargo clippy --workspace --all-targets -- -D warnings" \
  cargo clippy --workspace --all-targets -- -D warnings

if [[ "$SKIP_TESTS" -eq 0 ]]; then
  step "cargo test --workspace" \
    cargo test --workspace
else
  echo "==> Skipping cargo test (--skip-tests)"
fi

if [[ "$SKIP_MIRI_FLAG" -eq 1 || "${SKIP_MIRI:-}" == "1" ]]; then
  echo "==> Skipping Miri (--skip-miri / SKIP_MIRI=1)"
elif [[ "$(uname -s)" == "Linux" ]]; then
  if rustup run nightly rustc --version >/dev/null 2>&1; then
    step "cargo +nightly miri setup" cargo +nightly miri setup
    step "cargo +nightly miri test -p sandbox-linux" \
      cargo +nightly miri test -p sandbox-linux
  else
    echo "==> Skipping Miri (nightly toolchain not installed)"
  fi
else
  echo "==> Skipping Miri (not Linux)"
fi

echo ""
echo "Pre-commit gate passed."
