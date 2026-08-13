#!/usr/bin/env bash
# Offline portability guardrails: artifact ignore check + static path gate.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "== artifact ignore =="
probe_paths=(
  "run/out/manifest.json"
  "demo-home/searxng/out/manifest.json"
  "workflows/aoc-live/__pycache__/runtime.cpython-312.pyc"
  ".cursor/mcp.json"
  "deepseek__deepseek-v4-flash.regression-probe.json"
  "workflows/swebench-verified/artifacts/_probe"
  ".cursor/plans/probe.plan.md"
)
for rel in "${probe_paths[@]}"; do
  if ! git check-ignore -q "$rel"; then
    echo "Expected gitignore rule for $rel" >&2
    exit 1
  fi
  echo "ok ignored: $rel"
done

echo "== static absolute-path gate (tracked configs + VS Code examples) =="
# Word-boundary drive roots avoid matching https:// URL schemes.
pattern='(^|[^[:alnum:]_])[A-Za-z]:(/|\\)|/(Users|home)/'
hits=()
while IFS= read -r file; do
  [[ -z "$file" || ! -f "$file" ]] && continue
  if grep -Eiq "$pattern" "$file"; then
    hits+=("$file")
  fi
done < <(git ls-files -- \
  '**/agent-config*.yaml' \
  '**/agent-config*.yml' \
  'workflows/1c-dev/products/*.yaml.example' \
  'workflows/1c-dev/vscode/*.example.json' \
  '.cursor/mcp.json.example')

if ((${#hits[@]} > 0)); then
  echo "Tracked config files with machine-local path patterns:" >&2
  printf '  %s\n' "${hits[@]}" >&2
  echo "Static absolute-path gate failed (${#hits[@]} file(s))" >&2
  exit 1
fi
echo "ok: no disallowed absolute paths in tracked agent/product/VS Code configs"
echo "Portability guardrails passed."
