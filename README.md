# agent_Kuibysheff

[![CI](https://github.com/gazalievtimur/Agent-Kuibysheff/actions/workflows/ci.yml/badge.svg)](https://github.com/gazalievtimur/Agent-Kuibysheff/actions/workflows/ci.yml)

Minimal and reliable CLI agent worker in Rust.

## What it does

- Takes `--project-root` + `--agent`, a prompt, read-only input files, and an
  isolated home workspace (profile under `.kuibysheff/protected/agents/<id>/`).
- Runs an iterative agent loop against an OpenAI-compatible `/chat/completions` endpoint.
- Uses MCP servers over `stdio` or Streamable HTTP when the model requests tools.
- Can invoke MCP tools as ordered Event-MCP middleware around context and response stages.
- Serves ACP over stdio for IDE/bridges, and A2A 1.0 over HTTP for peer agents.
- Enforces hard stop limits: iterations, tokens, max duration, and optional max cost.
- Enforces an optional fail-closed `access` policy (tools, paths, `home.run`
  programs) and runs `home.run` inside an OS sandbox (Linux namespaces /
  Windows AppContainer) with no network.
- Produces a final JSON result with per-request token/cost accounting and optional
  AI/MCP logs.

The CLI is a worker, not an orchestrator. It never applies generated files to a
target repository. See [CONTRACT.md](CONTRACT.md) for the stable interface and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for architecture.

## Install

Prebuilt **Windows x86_64** and **Linux x86_64** binaries are on
[GitHub Releases](https://github.com/gazalievtimur/Agent-Kuibysheff/releases).
macOS and Linux aarch64 are **unsupported**. Archive names, checksums, glibc
baseline, and how to cut a release: [docs/RELEASING.md](docs/RELEASING.md).
Install / upgrade / uninstall (including user data under `.kuibysheff/`):
[docs/INSTALL.md](docs/INSTALL.md).

Or build from source (Rust MSRV **1.88**):

```powershell
git clone --recurse-submodules https://github.com/gazalievtimur/Agent-Kuibysheff.git
cd Agent-Kuibysheff
cargo build --release --bin kbshff
```

Breaking changes and migration notes (including required `access` in **0.2.0**):
[CHANGELOG.md](CHANGELOG.md). The VS Code extension is versioned independently
(see `extensions/vscode/package.json`).

## Quick start

```powershell
# Option A: interactive setup writes profile `.env` automatically
#   kbshff
# Option B: copy example and set OPENAI_API_KEY (or your api_key_env name)
Copy-Item .env.example .env
# edit .env

# Scaffold a protected profile, then import or edit via `config`
cargo run --bin kbshff -- init demo --project-root .
# Optional: import an existing YAML/settings bundle
# cargo run --bin kbshff -- config --project-root . --agent demo import --from ./settings --force

$env:OPENAI_API_KEY = "your_api_key"
cargo run --bin kbshff -- run `
  --project-root . `
  --agent demo `
  --prompt "Summarize the attached README into out/summary.md" `
  --files ./README.md
```

Or use the helper script (README summary template from
[`prompt-examples.md`](prompt-examples.md)):

```powershell
$env:OPENAI_API_KEY = "your_api_key"
.\run-demo.ps1
```

Runtime limit overrides:

```powershell
cargo run --bin kbshff -- run <required arguments> `
  --max-iterations 20 --max-tokens 25000 --max-duration-sec 180
```

See [`agent-config.example.yaml`](agent-config.example.yaml),
[`settings/`](settings/) (import sources), [`test-agents/`](test-agents/), and
[`prompt-examples.md`](prompt-examples.md). Profile storage is always under
`.kuibysheff/protected/agents/<id>/` — manage it with `config`, not by path flags.

## CLI

Interactive setup (no subcommand; TTY required):

```text
kbshff
```

Asks for the harness folder (default: current directory), checks or creates an
agent profile, prints connected/available resources, then offers further
configuration (MCP, provider, limits).

Worker (`run`):

```text
kbshff run \
  --project-root <DIR> \
  --agent <ID> \
  --prompt <TEXT> \
  [--home <REL_UNDER_KUIBYSHEFF>] \
  [--files <PATH>...] \
  [--run-id <ID>] \
  [--max-cost <CURRENCY:AMOUNT>]
```

Scaffold, probe, or manage settings (no storage path flags):

```text
kbshff init <agent-id> --project-root <DIR> [--force] [-i|--interactive]
kbshff check --project-root <DIR> --agent <ID>
kbshff config --project-root <DIR> --agent <ID> import --from <PATH> [--force]
kbshff config --project-root <DIR> --agent <ID> show
```

ACP stdio server (VS Code, messengers, mail bridges):

```text
kbshff acp \
  --agent <ID> \
  [--project-root <DIR>]
```

`stdin`/`stdout` are ACP JSON-RPC only; put diagnostics on `stderr`. Prefer one
long-lived process per agent. Full bridge contract:
[CONTRACT.md](CONTRACT.md#acp-ide-messengers-mail-bridges).
VS Code extension: [extensions/vscode/README.md](extensions/vscode/README.md).

A2A HTTP server (peer agents):

```text
kbshff a2a \
  --project-root <DIR> \
  --agent <ID> \
  [--bind 127.0.0.1:8787] \
  [--token-env A2A_TOKEN]
```

Serves Agent Card + JSON-RPC/REST for A2A 1.0. Defaults to loopback. See
[CONTRACT.md](CONTRACT.md#a2a-peer-agents).

## Local eval / copy-units

Example orchestrators and live LLM regression harnesses live under `workflows/`
**locally only** (gitignored). They are not part of the published product tree.
Restore from git history when you need them for testing:

```bash
git checkout <commit-before-untrack> -- workflows
```

Opt-in gates (require restored copy-units where applicable):

- SWE-bench: `scripts/swebench-regression.*` / `check.* -Swebench`
- Security sandbox: `scripts/security-regression.*` / `check.* -Security`
- Scale-FS: `scripts/scale-fs-regression.*` / `check.* -ScaleFs`
- AoC: `scripts/aoc-regression.*` / `check.* -Aoc` (uses `local/aoc-bank`, not `workflows/`)

Agent profile templates: [test-agents/README.md](test-agents/README.md).
Local banks and run artifacts: [local/README.md](local/README.md).

## Configuration

- Runtime config (including required `access` policy):
  [`agent-config.example.yaml`](agent-config.example.yaml). Prefer fail-closed
  tools, path grants, and program aliases; permissive FS only via explicit
  `access: { mode: legacy }`. Details: [CONTRACT.md](CONTRACT.md#access-policy-fail-closed).
- Billing / cost budgets: [docs/BILLING.md](docs/BILLING.md)
- Event-MCP ordered middleware on MCP `tools/call`:
  [docs/EVENT_MCP.md](docs/EVENT_MCP.md)
- Logging sinks and chat history: [docs/LOGGING.md](docs/LOGGING.md)
- `RunOutput` JSON and exit codes: [docs/OUTPUT.md](docs/OUTPUT.md)
- Skills DSL `allowed_tools` intersect with access policy:
  [CONTRACT.md](CONTRACT.md#access-policy-fail-closed)

Model context-window pruning lives under `provider.history` (defaults
`max_tail_messages: 30`, `max_chars: 200000`) and is independent of
`limits.max_tokens`. Optional `provider.effort` sets reasoning effort for
compatible models (`reasoning_effort` in the Chat Completions request).

## Documentation

Full index: [docs/README.md](docs/README.md).

Security reports: [SECURITY.md](SECURITY.md). Support policy: [SUPPORT.md](SUPPORT.md).
Contributing / CoC: [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

Engineering backlog notes (not product docs):
[docs/architecture-review/](docs/architecture-review/), [docs/FURTHER_FIXES.md](docs/FURTHER_FIXES.md).

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE)
for copyright. Contributions are under the same terms; see
[CONTRIBUTING.md](CONTRIBUTING.md).

The names **Kuibysheff** and **agent_Kuibysheff** are not licensed for
trademark use beyond reasonable attribution of origin.

Canonical product / binary / repository names: **Kuibysheff**,
`agent_Kuibysheff` (crate), `kbshff` (CLI binary), and `Agent-Kuibysheff`.
Local folder spellings such as `Agent Kuibyshev` are legacy path aliases only
and must not be used for discovery.
