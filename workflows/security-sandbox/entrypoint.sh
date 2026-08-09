#!/usr/bin/env bash
# Security sandbox lab entrypoint (runs inside outer Docker).
#
# - Enables unprivileged userns when possible (lab-only).
# - Optional egress allowlist via iptables (PROVIDER_EGRESS_HOST).
# - REQUIRE_LINUX_SANDBOX preflight before any LLM call.
# - NEVER expects docker.sock; NEVER sets KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP.
set -euo pipefail

echo "== security-sandbox: lab entrypoint =="

if [[ -n "${KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP:-}" ]]; then
  echo "ERROR: KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP must not be set for security regression." >&2
  exit 1
fi

export SECURITY_IN_DOCKER=1
export PYTHONUNBUFFERED=1
mkdir -p /canary
chmod 755 /canary

# Best-effort: allow mounts inside unprivileged userns (reverts with container).
if [[ -w /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]]; then
  echo 0 > /proc/sys/kernel/apparmor_restrict_unprivileged_userns || true
fi
if [[ -w /proc/sys/kernel/unprivileged_userns_clone ]]; then
  echo 1 > /proc/sys/kernel/unprivileged_userns_clone || true
fi

if [[ -f /proc/sys/kernel/unprivileged_userns_clone ]]; then
  echo "unprivileged_userns_clone=$(cat /proc/sys/kernel/unprivileged_userns_clone)"
fi
if [[ -f /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]]; then
  echo "apparmor_restrict_unprivileged_userns=$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns)"
fi

setup_egress_allowlist() {
  local host="${PROVIDER_EGRESS_HOST:-}"
  if [[ -z "$host" ]]; then
    echo "egress allowlist: skipped (PROVIDER_EGRESS_HOST unset)"
    return 0
  fi
  if ! command -v iptables >/dev/null 2>&1; then
    echo "egress allowlist: iptables missing; skipped" >&2
    return 0
  fi
  echo "egress allowlist: allowing DNS + $host (443/80), dropping other egress"
  iptables -C OUTPUT -o lo -j ACCEPT 2>/dev/null || iptables -I OUTPUT -o lo -j ACCEPT
  iptables -C OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT 2>/dev/null \
    || iptables -I OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  iptables -C OUTPUT -p udp --dport 53 -j ACCEPT 2>/dev/null || iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
  iptables -C OUTPUT -p tcp --dport 53 -j ACCEPT 2>/dev/null || iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
  local ip
  for ip in $(getent ahosts "$host" | awk '{print $1}' | sort -u); do
    iptables -C OUTPUT -d "$ip" -p tcp --dport 443 -j ACCEPT 2>/dev/null \
      || iptables -A OUTPUT -d "$ip" -p tcp --dport 443 -j ACCEPT
    iptables -C OUTPUT -d "$ip" -p tcp --dport 80 -j ACCEPT 2>/dev/null \
      || iptables -A OUTPUT -d "$ip" -p tcp --dport 80 -j ACCEPT
  done
  iptables -C OUTPUT -p tcp -j DROP 2>/dev/null || iptables -A OUTPUT -p tcp -j DROP || true
}

cd /work

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/kuibysheff-security-target}"
mkdir -p "$CARGO_TARGET_DIR"

# Build + sandbox preflight need crates.io / static.rust-lang.org — apply egress
# allowlist only after those steps, before the live LLM eval.
echo "== security-sandbox: build release agent =="
cargo build --release -p agent_Kuibysheff
AGENT_BIN="$CARGO_TARGET_DIR/release/agent_Kuibysheff"
chmod +x "$AGENT_BIN"
cp "$AGENT_BIN" /usr/local/bin/agent_Kuibysheff

echo "== security-sandbox: OS sandbox preflight (REQUIRE_LINUX_SANDBOX=1) =="
export REQUIRE_LINUX_SANDBOX=1
cargo test -p sandbox-linux --test namespaces echo_under_grants -- --exact --nocapture --test-threads=1

setup_egress_allowlist || echo "egress allowlist: setup failed (continuing; sandbox still required)"

echo "== security-sandbox: run regression eval =="
exec bash /work/scripts/security-regression.sh --already-in-lab --agent-bin /usr/local/bin/agent_Kuibysheff "$@"
