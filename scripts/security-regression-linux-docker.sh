#!/usr/bin/env bash
# Build/run the security lab image and execute entrypoint (no docker.sock).
#
# Usage:
#   ./scripts/security-regression-linux-docker.sh
#   ./scripts/security-regression-linux-docker.sh --task-id escape-fs-sibling-01
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# shellcheck source=import-dotenv.sh
source "$SCRIPT_DIR/import-dotenv.sh"
import_dotenv "$REPO_ROOT/.env"

IMAGE="${SECURITY_LAB_IMAGE:-kuibysheff-security-lab}"
CONFIG=""
FORWARD=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      CONFIG="${2:-}"
      shift 2
      ;;
    --task-id|--agent-bin)
      FORWARD+=("$1" "${2:-}")
      shift 2
      ;;
    --require-limits|--require-cost-limit)
      FORWARD+=("$1")
      shift
      ;;
    --image)
      IMAGE="${2:-}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,10p' "$0"
      exit 0
      ;;
    *)
      FORWARD+=("$1")
      shift
      ;;
  esac
done

if ! command -v docker >/dev/null 2>&1; then
  echo "docker CLI is required for the security lab container." >&2
  exit 1
fi

if [[ -z "$CONFIG" ]]; then
  if [[ -f "$REPO_ROOT/agent-config.local.yaml" ]]; then
    CONFIG="$REPO_ROOT/agent-config.local.yaml"
  else
    CONFIG="$REPO_ROOT/test-agents/security-probe/agent-config.example.yaml"
  fi
fi
if [[ ! -f "$CONFIG" ]]; then
  echo "Config not found: $CONFIG" >&2
  exit 1
fi

CONFIG_TEXT="$(cat "$CONFIG")"
API_KEY_ENV="$(python3 -c '
import re,sys
text=sys.stdin.read()
for pat in [r"(?m)^\s*api_key_env:\s*\"([^\"]+)\"", r"(?m)^\s*api_key_env:\s*'\''([^'\'']+)'\''", r"(?m)^\s*api_key_env:\s*([A-Za-z_][A-Za-z0-9_]*)"]:
    m=re.search(pat,text)
    if m:
        print(m.group(1).strip()); break
else:
    print("OPENAI_API_KEY")
' <<<"$CONFIG_TEXT")"
API_KEY_VALUE="${!API_KEY_ENV:-}"
if [[ -z "$API_KEY_VALUE" ]]; then
  echo "Security lab requires $API_KEY_ENV in the environment or .env" >&2
  exit 1
fi

PROVIDER_HOST="$(python3 -c '
import re,sys
from urllib.parse import urlparse
text=sys.stdin.read()
m=re.search(r"(?m)^\s*base_url:\s*[\"'\'']?([^\"'\''#\r\n]+)", text)
url=(m.group(1).strip() if m else "https://api.openai.com/v1")
print(urlparse(url if "://" in url else "https://"+url).hostname or "api.openai.com")
' <<<"$CONFIG_TEXT")"

# Config path as seen in container.
CONFIG_ABS="$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")"
case "$CONFIG_ABS" in
  "$REPO_ROOT"/*) CONFIG_IN="/work/${CONFIG_ABS#"$REPO_ROOT"/}" ;;
  *)
    echo "Config must live under the repo so it is visible at /work: $CONFIG" >&2
    exit 1
    ;;
esac

echo "Building security lab image: $IMAGE"
docker build -f "$REPO_ROOT/workflows/security-sandbox/Dockerfile" -t "$IMAGE" "$REPO_ROOT"

echo "Running security lab (privileged userns lab; NO docker.sock)..."
docker run --rm --privileged \
  -v "$REPO_ROOT:/work" \
  -v kuibysheff-security-cargo:/tmp/kuibysheff-security-target \
  -e "${API_KEY_ENV}=${API_KEY_VALUE}" \
  -e "PROVIDER_EGRESS_HOST=${PROVIDER_HOST}" \
  -e "SECURITY_IN_DOCKER=1" \
  -e "CARGO_TARGET_DIR=/tmp/kuibysheff-security-target" \
  -w /work \
  "$IMAGE" \
  bash /work/workflows/security-sandbox/entrypoint.sh \
  --config "$CONFIG_IN" \
  "${FORWARD[@]+"${FORWARD[@]}"}"
