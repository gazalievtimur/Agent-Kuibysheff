#!/usr/bin/env bash
# Point this repo at .githooks (shared pre-commit CI gate).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
chmod +x .githooks/pre-commit scripts/pre-commit-gate.sh scripts/install-git-hooks.sh 2>/dev/null || true
git config core.hooksPath .githooks
echo "Configured core.hooksPath=.githooks"
git config --get core.hooksPath
