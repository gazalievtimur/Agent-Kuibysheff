#!/usr/bin/env bash
# CI-parity gate used by git pre-commit / Cursor hooks.
#
# Mirrors the fast CI jobs, plus local supply-chain:
#   - cargo fmt --all -- --check
#   - cargo clippy --workspace --all-targets -- -D warnings
#   - cargo deny check (skip with --skip-deny / SKIP_DENY=1)
#   - cargo test --workspace
#   - cargo +nightly miri test -p sandbox-linux --lib (Linux; skip with --skip-miri)
#
# Bypass: SKIP_PRECOMMIT=1
set -euo pipefail

SKIP_MIRI_FLAG=0
SKIP_TESTS=0
SKIP_DENY_FLAG=0

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
    --skip-deny|-SkipDeny)
      SKIP_DENY_FLAG=1
      shift
      ;;
    -h|--help)
      sed -n '2,14p' "$0"
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

if [[ "$SKIP_DENY_FLAG" -eq 1 || "${SKIP_DENY:-}" == "1" ]]; then
  echo "==> Skipping cargo deny (--skip-deny / SKIP_DENY=1)"
else
  if ! cargo deny --version >/dev/null 2>&1; then
    echo "cargo-deny is not installed. Install with: cargo install --locked cargo-deny" >&2
    exit 1
  fi
  step "cargo deny check" cargo deny check
fi

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
    step "cargo +nightly miri test -p sandbox-linux --lib" \
      cargo +nightly miri test -p sandbox-linux --lib
  else
    echo "==> Skipping Miri (nightly toolchain not installed)"
  fi
else
  echo "==> Skipping Miri (not Linux)"
fi

echo ""
echo "Pre-commit gate passed."
