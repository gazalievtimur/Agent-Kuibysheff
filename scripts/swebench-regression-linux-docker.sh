#!/usr/bin/env bash
# Inner entrypoint: run SWE-bench regression as Linux ELF inside a container.
#
# Designed for Docker Desktop on Windows/macOS hosts that lack a native Linux
# toolchain. Expects the repo at /work and the host Docker socket mounted.
#
# Do not call this on a native Linux host — use ./scripts/swebench-regression.sh.
#
# Typical host launchers:
#   ./scripts/swebench-regression-linux-docker.ps1   (Windows)
#   docker run --rm -v "$PWD:/work" -v /var/run/docker.sock:/var/run/docker.sock \
#     -e POLZA_API_KEY -e KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1 \
#     rust:1-bookworm bash /work/scripts/swebench-regression-linux-docker.sh
set -euo pipefail

export PATH="/usr/local/cargo/bin:/usr/local/bin:${PATH}"

# Nested Docker Desktop kernels often lack clone3 for crates/sandbox-linux.
# Stdio MCP for the workspace server needs this escape hatch inside the runner.
export KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP="${KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP:-1}"

# Keep Linux artifacts off the Windows-mounted target/ so PE and ELF do not mix.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/kuibysheff-target}"

DOCKER_STATIC_VERSION="${DOCKER_STATIC_VERSION:-27.5.1}"
REPO_ROOT="${REPO_ROOT:-/work}"
cd "$REPO_ROOT"

if [[ ! -d "$REPO_ROOT/workflows/swebench-verified" ]]; then
  cat >&2 <<'EOF'
workflows/swebench-verified not found (gitignored copy-unit).

Restore from git history for local testing, for example:
  git checkout HEAD~1 -- workflows
  # or: git checkout <commit-before-untrack> -- workflows
EOF
  exit 1
fi

# Host checkout on Windows often has CRLF shebangs; strip before bash executes them.
if command -v sed >/dev/null 2>&1; then
  find scripts workflows/swebench-verified -type f -name '*.sh' -print0 \
    | xargs -0 sed -i 's/\r$//'
fi

echo "== linux-docker: install runner deps =="
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  python3 python3-pip curl ca-certificates file >/dev/null

if ! command -v docker >/dev/null 2>&1; then
  echo "Installing static Docker CLI ${DOCKER_STATIC_VERSION}..."
  curl -fsSL "https://download.docker.com/linux/static/stable/x86_64/docker-${DOCKER_STATIC_VERSION}.tgz" \
    | tar -xz --strip-components=1 -C /usr/local/bin docker/docker
fi
docker version --format '{{.Server.Os}}' >/dev/null

python3 -m pip install -q --break-system-packages \
  -r workflows/swebench-verified/requirements.txt

echo "== linux-docker: build release agent (CARGO_TARGET_DIR=$CARGO_TARGET_DIR) =="
cargo --version
cargo build --release
install -m 755 "$CARGO_TARGET_DIR/release/agent_Kuibysheff" /usr/local/bin/agent_Kuibysheff
if ! file /usr/local/bin/agent_Kuibysheff | grep -q ELF; then
  echo "expected Linux ELF at /usr/local/bin/agent_Kuibysheff" >&2
  file /usr/local/bin/agent_Kuibysheff >&2 || true
  exit 1
fi
hash -r
agent_Kuibysheff --help >/dev/null

chmod +x scripts/*.sh workflows/swebench-verified/run.sh 2>/dev/null || true

echo "== linux-docker: swebench-regression =="
# Explicit --agent-bin avoids resolving a host-mounted Windows .exe from target/release.
exec bash scripts/swebench-regression.sh --agent-bin /usr/local/bin/agent_Kuibysheff "$@"
