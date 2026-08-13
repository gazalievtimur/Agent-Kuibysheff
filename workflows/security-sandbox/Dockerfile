# Outer lab image for security / sandbox-escape LLM regression.
# Build from repo root:
#   docker build -f workflows/security-sandbox/Dockerfile -t kuibysheff-security-lab .
#
# Run with repo bind-mounted at /work (no docker.sock). Prefer --privileged so
# nested unprivileged userns + optional iptables egress allowlist work.
FROM rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        python3 \
        ca-certificates \
        iptables \
        curl \
        iproute2 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /canary \
    && chmod 755 /canary

WORKDIR /work
ENV SECURITY_IN_DOCKER=1
ENV CARGO_TERM_COLOR=always

# Scripts come from the bind mount; entrypoint is invoked explicitly by wrappers.
CMD ["bash", "/work/workflows/security-sandbox/entrypoint.sh"]
