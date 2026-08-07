# agent_Kuibysheff

[![CI](https://github.com/gybson63/Agent-Kuibysheff/actions/workflows/ci.yml/badge.svg)](https://github.com/gybson63/Agent-Kuibysheff/actions/workflows/ci.yml)

Minimal and reliable CLI agent worker in Rust.

## What it does

- Takes runtime config, an agent settings directory, a prompt, read-only input
  files, and an isolated home workspace.
- Runs an iterative agent loop against an OpenAI-compatible `/chat/completions` endpoint.
- Uses MCP servers over `stdio` or Streamable HTTP when the model requests tools.
- Can invoke MCP tools as ordered Event-MCP middleware around context and response stages.
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

Prebuilt Windows and Linux binaries are on
[GitHub Releases](https://github.com/gybson63/Agent-Kuibysheff/releases).
Archive names, checksums, and how to cut a release: [docs/RELEASING.md](docs/RELEASING.md).

Or build from source (Rust MSRV **1.88**):

```powershell
cargo build --release --bin agent_Kuibysheff
```

## Quick start

```powershell
# Option A: .env file in repo root (loaded automatically)
Copy-Item .env.example .env
# edit .env and set POLZA_API_KEY=...

# Option B: inline key in gitignored agent-config.local.yaml
# provider:
#   api_key: "your_key"

# Option C: explicit environment variable
$env:POLZA_API_KEY = "your_api_key"
cargo run --bin agent_Kuibysheff -- run `
  --config ./agent-config.local-demo.yaml `
  --settings-dir ./settings `
  --prompt "Summarize the attached README into out/summary.md" `
  --home ./demo-home `
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
cargo run --bin agent_Kuibysheff -- run <required arguments> `
  --max-iterations 20 --max-tokens 25000 --max-duration-sec 180
```

See [`agent-config.example.yaml`](agent-config.example.yaml),
[`settings/`](settings/), [`test-agents/`](test-agents/), and
[`prompt-examples.md`](prompt-examples.md).

## CLI

Worker (`run`):

```text
agent_Kuibysheff run \
  --config <FILE> \
  --settings-dir <DIR> \
  --prompt <TEXT> \
  --home <DIR> \
  [--project-root <DIR>] \
  [--files <PATH>...] \
  [--run-id <ID>] \
  [--max-cost <CURRENCY:AMOUNT>]
```

With `--project-root`, relative `--config` / `--settings-dir` / `--home` resolve
under `{project-root}/.kuibysheff/`.

Scaffold a profile, or probe config before a run:

```text
agent_Kuibysheff init <agent-id> [--path DIR] [--force] [-i|--interactive]
agent_Kuibysheff check --config <FILE> [--settings-dir <DIR>]
```

ACP stdio server (VS Code, messengers, mail bridges):

```text
agent_Kuibysheff acp \
  --config <FILE> \
  --settings-dir <DIR> \
  --home <DIR> \
  [--project-root <DIR>]
```

`stdin`/`stdout` are ACP JSON-RPC only; put diagnostics on `stderr`. Prefer one
long-lived process per config. Full bridge contract:
[CONTRACT.md](CONTRACT.md#acp-ide-messengers-mail-bridges).
VS Code extension: [extensions/vscode/README.md](extensions/vscode/README.md).

## Example workflows

- **1C development conveyor** (Jira/Confluence → analysis → coder → CFE):
  [workflows/1c-dev/README.md](workflows/1c-dev/README.md),
  [workflows/1c-dev/VSCODE.md](workflows/1c-dev/VSCODE.md)
- **Live Advent of Code** (ACP singleton: download → solve → submit → retry):
  [workflows/aoc-live/README.md](workflows/aoc-live/README.md)
- **SWE-bench Verified** (Docker patches + official harness):
  [workflows/swebench-verified/README.md](workflows/swebench-verified/README.md)

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
`limits.max_tokens`.

## Documentation

Full index: [docs/README.md](docs/README.md).

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE)
for copyright. Contributions are under the same terms; see
[CONTRIBUTING.md](CONTRIBUTING.md).

The names **Kuibysheff** and **agent_Kuibysheff** are not licensed for
trademark use beyond reasonable attribution of origin.
