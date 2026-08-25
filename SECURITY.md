# Security Policy

## Supported Versions

| Version | Supported |
| --- | --- |
| 0.2.x | Yes |
| < 0.2 | No |

Report issues against the latest 0.2.x release whenever possible.

## Reporting a Vulnerability

**Do not** open a public GitHub issue for security vulnerabilities, and do not
include exploitable details, PoCs that enable abuse, or secrets in public
channels.

Prefer one of these private channels:

1. **GitHub Private Vulnerability Reporting** (preferred):  
   https://github.com/gazalievtimur/Agent-Kuibysheff/security/advisories/new  
   Enable *Private vulnerability reporting* in the repository Security settings
   if the link is unavailable.
2. **Email:** `gazalievtimur+kuibysheff-security@users.noreply.github.com`  
   Maintainers should replace this with a dedicated security mailbox before
   public Go. Until then, Private Vulnerability Reporting is the required path.

Please include:

- affected version / commit;
- environment (OS, sandbox mode, MCP transport);
- impact and reproduction steps sufficient for triage;
- whether you are requesting coordinated disclosure credit.

### Response

- We aim to acknowledge private reports within **7 days**.
- We coordinate a fix and disclosure timeline with the reporter when practical.
- Public discussion should wait until a fix or mitigation is available, unless
  the issue is already widely known.

## Trust boundaries (summary)

`agent_Kuibysheff` is a local worker. Treat the following as security-sensitive:

- **MCP stdio / Streamable HTTP** — tools run with the privileges of the process
  and configured MCP servers; untrusted tool output can influence the model.
- **Network egress** — provider HTTP and MCP HTTP leave the host; `home.run` OS
  sandboxes are intended to have **no network**.
- **A2A inbound HTTP** (`kbshff a2a`) — new listen surface for peer agents.
  Default bind is loopback (`127.0.0.1`). Do not expose on a public interface
  without a reverse proxy and `--token-env` (Bearer). The Agent Card endpoint
  is intentionally public for discovery; `/jsonrpc` and `/rest` enforce the
  token when configured. Tokens must not appear in logs.
- **`KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP`** — opt-in escape hatch; do not enable in
  untrusted environments.
- **Protected store** (`.kuibysheff/protected/`) — agent profiles and secrets
  layouts; keep out of shared untrusted trees.
- **OS sandbox limits** — Linux namespaces/seccomp (x86_64) and Windows
  AppContainer constrain `home.run` payloads; they are not a full VM isolation
  boundary and do not replace host hardening.

See [CONTRACT.md](CONTRACT.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
for operational detail. Opt-in security LLM regression lives under the local
`workflows/security-sandbox/` copy-unit (gitignored; restore from git history).

## Non-security bugs

Use GitHub Issues for crashes, incorrect behavior, and documentation problems
that are not security-sensitive. See [SUPPORT.md](SUPPORT.md).
