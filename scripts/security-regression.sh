#!/usr/bin/env bash
# Prerequisites + security / sandbox-escape LLM regression (Linux native or lab).
#
# Pass = containment (canaries untouched / no exfil). Requires a working OS sandbox.
# Never sets KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP.
#
# Usage:
#   ./scripts/security-regression.sh
#   ./scripts/security-regression.sh --task-id escape-fs-sibling-01
#   ./scripts/security-regression.sh --config path/to/config.yaml
#   ./scripts/security-regression.sh --already-in-lab --agent-bin /usr/local/bin/agent_Kuibysheff
#
# On Windows hosts use:
#   .\scripts\security-regression-linux-docker.ps1
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# shellcheck source=import-dotenv.sh
source "$SCRIPT_DIR/import-dotenv.sh"
import_dotenv "$REPO_ROOT/.env"

CONFIG=""
AGENT_BIN=""
ALREADY_IN_LAB=0
REQUIRE_LIMITS=0
REQUIRE_COST_LIMIT=0
TASK_IDS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      CONFIG="${2:-}"
      shift 2
      ;;
    --task-id)
      TASK_IDS+=("${2:-}")
      shift 2
      ;;
    --agent-bin)
      AGENT_BIN="${2:-}"
      shift 2
      ;;
    --already-in-lab)
      ALREADY_IN_LAB=1
      shift
      ;;
    --require-limits)
      REQUIRE_LIMITS=1
      shift
      ;;
    --require-cost-limit)
      REQUIRE_COST_LIMIT=1
      shift
      ;;
    -h|--help)
      sed -n '2,16p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -n "${KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP:-}" ]]; then
  echo "Refusing security regression with KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP set." >&2
  exit 1
fi

BANK_DIR="$REPO_ROOT/local/security-bank"
if [[ ! -d "$BANK_DIR" ]]; then
  cat >&2 <<EOF
Security regression bank not found: $BANK_DIR

Copy the example (gitignored working copy):
  cp -R ./local/security-bank.example ./local/security-bank
EOF
  exit 1
fi

TASK_COUNT="$(find "$BANK_DIR" -maxdepth 1 -type f -name '*.json' | wc -l | tr -d ' ')"
if [[ "$TASK_COUNT" -eq 0 ]]; then
  echo "Security regression bank is empty: $BANK_DIR" >&2
  exit 1
fi

if [[ -z "$CONFIG" ]]; then
  LOCAL_CONFIG="$REPO_ROOT/agent-config.local.yaml"
  EXAMPLE_CONFIG="$REPO_ROOT/test-agents/security-probe/agent-config.example.yaml"
  if [[ -f "$LOCAL_CONFIG" ]]; then
    CONFIG="$LOCAL_CONFIG"
  else
    CONFIG="$EXAMPLE_CONFIG"
  fi
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "Security regression config not found: $CONFIG" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1; then
  echo "Security regression requires python3 on PATH." >&2
  exit 1
fi
PY=python3
command -v python3 >/dev/null 2>&1 || PY=python

CONFIG_TEXT="$(cat "$CONFIG")"
API_KEY_ENV="$(
  printf '%s' "$CONFIG_TEXT" | PYTHONPATH="$REPO_ROOT/workflows/security-sandbox${PYTHONPATH:+:$PYTHONPATH}" \
    "$PY" -c 'from security_lib import yaml_scalar; import sys; print(yaml_scalar(sys.stdin.read(), "api_key_env", "OPENAI_API_KEY"))'
)"

if [[ -z "${!API_KEY_ENV:-}" ]]; then
  cat >&2 <<EOF
Security regression requires a provider API key via environment.

Set:
  - environment variable $API_KEY_ENV
  - $API_KEY_ENV in $REPO_ROOT/.env

Inline provider.api_key in config is rejected by ConfigSafetyValidator.
EOF
  exit 1
fi

if [[ "$ALREADY_IN_LAB" -eq 0 ]]; then
  echo "Security regression: bank=$BANK_DIR tasks=$TASK_COUNT config=$CONFIG"
  echo "Building release agent (sandboxed home.run; sandbox probe required)..."
  cargo build --release
  # Native Linux preflight — fail closed if userns sandbox unavailable.
  if [[ "$(uname -s)" == "Linux" ]]; then
    echo "OS sandbox preflight (REQUIRE_LINUX_SANDBOX=1)..."
    REQUIRE_LINUX_SANDBOX=1 cargo test -p sandbox-linux --test namespaces echo_under_grants \
      -- --exact --nocapture --test-threads=1
  fi
  AGENT_BIN="${AGENT_BIN:-$REPO_ROOT/target/release/agent_Kuibysheff}"
else
  echo "Security regression (lab): bank=$BANK_DIR tasks=$TASK_COUNT config=$CONFIG"
  if [[ -z "$AGENT_BIN" ]]; then
    echo "--already-in-lab requires --agent-bin" >&2
    exit 2
  fi
fi

EVAL_ARGS=(
  --repo-root "$REPO_ROOT"
  --bank-dir "$BANK_DIR"
  --config "$CONFIG"
  --settings-dir "$REPO_ROOT/test-agents/security-probe"
  --agent-bin "$AGENT_BIN"
)
if [[ "$REQUIRE_LIMITS" -eq 1 ]]; then
  EVAL_ARGS+=(--require-limits)
fi
if [[ "$REQUIRE_COST_LIMIT" -eq 1 ]]; then
  EVAL_ARGS+=(--require-cost-limit)
fi
for tid in "${TASK_IDS[@]+"${TASK_IDS[@]}"}"; do
  EVAL_ARGS+=(--task-id "$tid")
done

set +e
"$PY" "$REPO_ROOT/workflows/security-sandbox/security_eval.py" "${EVAL_ARGS[@]}"
EVAL_RC=$?
set -e

LATEST_PTR="$REPO_ROOT/local/security-runs/LATEST"
if [[ ! -f "$LATEST_PTR" ]]; then
  echo "Security regression: LATEST pointer missing under local/security-runs" >&2
  exit 1
fi
REPORT_REF="$(tr -d '\r\n' <"$LATEST_PTR")"
if [[ "$REPORT_REF" = /* ]]; then
  REPORT="$REPORT_REF"
else
  REPORT="$REPO_ROOT/$REPORT_REF"
fi
if [[ -z "$REPORT" || ! -f "$REPORT" ]]; then
  echo "Security regression: report.json not found: $REPORT_REF" >&2
  exit 1
fi

"$PY" "$REPO_ROOT/workflows/security-sandbox/assert_regression.py" "$REPORT"
if [[ "$EVAL_RC" -ne 0 ]]; then
  exit "$EVAL_RC"
fi

echo "Security regression passed."
