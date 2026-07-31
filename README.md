# agent_Kuibyshev

[![CI](https://github.com/gybson63/Agent-Kuibyshev/actions/workflows/ci.yml/badge.svg)](https://github.com/gybson63/Agent-Kuibyshev/actions/workflows/ci.yml)

Minimal and reliable CLI agent worker in Rust.

## What it does

- Takes runtime config, an agent settings directory, a prompt, read-only input
  files, and an isolated home workspace.
- Runs an iterative agent loop against an OpenAI-compatible `/chat/completions` endpoint.
- Uses MCP servers over `stdio` or Streamable HTTP when the model requests tools.
- Enforces hard stop limits: iterations, tokens, and max duration.
- Enforces an optional fail-closed `access` policy (tools, paths, `home.run`
  programs) and runs `home.run` inside an OS sandbox (Linux namespaces /
  Windows AppContainer) with no network.
- Produces a final JSON result with usage stats and optional AI/MCP logs.

The CLI is a worker, not an orchestrator. It never applies generated files to a
target repository. See [CONTRACT.md](CONTRACT.md) for the stable interface an
external orchestrator must implement, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
for a high-level architectural overview.

For a multi-stage **1C development conveyor** (Jira/Confluence intake → analysis
→ coder → CFE packaging), see [workflows/1c-dev/README.md](workflows/1c-dev/README.md)
and `scripts/1c-dev-run.ps1`.

## Inputs

Worker (`run`):

```text
agent_Kuibyshev run \
  --config <FILE> \
  --settings-dir <DIR> \
  --prompt <TEXT> \
  --home <DIR> \
  [--files <PATH>...]
```

Scaffold a new agent profile:

```text
agent_Kuibyshev help
agent_Kuibyshev init <agent-id> [--path DIR] [--force] [-i|--interactive]
```

Creates `./<agent-id>/` (or `--path`) with settings files and
`agent-config.example.yaml`. `--interactive` prompts for provider and limits.

Check that configured resources are reachable before a run:

```text
agent_Kuibyshev check --config <FILE> [--settings-dir <DIR>]
```

Probes the provider API key/HTTP endpoint, each MCP server, access paths and
programs, logging dir, optional settings files, and the OS sandbox when
`home.run` programs are configured. Exit code is non-zero if any probe fails.

## Releases

Prebuilt binaries for Windows and Linux are published on
[GitHub Releases](https://github.com/gybson63/Agent-Kuibyshev/releases)
when a version tag is pushed.

| Platform | Archive |
| --- | --- |
| Windows x86_64 | `agent_Kuibyshev-vX.Y.Z-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `agent_Kuibyshev-vX.Y.Z-x86_64-unknown-linux-gnu.zip` |

Each archive contains the `agent_Kuibyshev` binary. A matching
`.zip.sha256` checksum file is attached to the release.

PowerShell (Windows):

```powershell
# After downloading and extracting the zip:
.\agent_Kuibyshev-v0.1.0-x86_64-pc-windows-msvc.exe --help
```

Bash (Linux):

```bash
# After downloading and extracting the zip:
chmod +x ./agent_Kuibyshev-v0.1.0-x86_64-unknown-linux-gnu
./agent_Kuibyshev-v0.1.0-x86_64-unknown-linux-gnu --help
```

To cut a release from a commit on `main`:

```bash
# Keep Cargo.toml version in sync with the tag when bumping.
git tag v0.1.0
git push origin v0.1.0
```

The [Release](.github/workflows/release.yml) workflow builds
`--release --locked --bin agent_Kuibyshev` on `windows-latest` and
`ubuntu-latest`, then uploads the zips to a GitHub Release for that tag.

## Quick start

Prerequisites:
- Rust toolchain (`cargo`), MSRV **1.86**
- Or a prebuilt binary from [Releases](#releases)

PowerShell commands:

```powershell
# Option A: .env file in repo root (loaded automatically)
Copy-Item .env.example .env
# edit .env and set POLZA_API_KEY=...

# Option B: inline key in gitignored agent-config.local.yaml
# provider:
#   api_key: "your_key"

# Option C: explicit environment variable
$env:POLZA_API_KEY = "your_api_key"
cargo run --bin agent_Kuibyshev -- run `
  --config ./agent-config.local-demo.yaml `
  --settings-dir ./settings `
  --prompt "Summarize the attached README into out/summary.md" `
  --home ./demo-home `
  --files ./README.md
```

Or use helper script (uses the README summary template from
[`prompt-examples.md`](prompt-examples.md)):

```powershell
$env:OPENAI_API_KEY = "your_api_key"
.\run-demo.ps1
```

This run uses:
- OpenAI-compatible provider from config
- Built-in repository research tools (`local_tools.search_docs`, `local_tools.read_file`)
- Built-in filesystem tools restricted to `./demo-home`
- Agent behavior from the `./settings` directory
- JSONL event logs and tracing output in `~/.agent-kuibyshev/logs` by default

Runtime limit overrides remain available:

```powershell
cargo run --bin agent_Kuibyshev -- run <required arguments> `
  --max-iterations 20 --max-tokens 25000 --max-duration-sec 180
```

Model context-window pruning is configured under `provider.history` (defaults
`max_tail_messages: 30`, `max_chars: 200000`). Raise these for long-context
models; they are independent of `limits.max_tokens` (the run stop budget).

See [`agent-config.example.yaml`](agent-config.example.yaml) for runtime config
(including the optional `access` policy),
[`settings/`](settings/) for the settings layout,
[`test-agents/`](test-agents/) for specialized test agent profiles, and
[`prompt-examples.md`](prompt-examples.md) for ready-to-use `--prompt` templates.

## Access policy and sandbox

Omit `access` for legacy filesystem behavior (`home.run` stays hidden). Add an
`access` block for fail-closed tools, path grants, and program aliases. Details
and migration steps: [CONTRACT.md](CONTRACT.md#access-policy-fail-closed).

`home.run` always uses the host OS sandbox (no network). Platform notes:

- Windows AppContainer integration tests: `cargo test -p sandbox-windows --test appcontainer`
- Linux namespaces: [crates/sandbox-linux/TESTING.md](crates/sandbox-linux/TESTING.md)

## Rust skills

The project bundles [rust-skills](https://github.com/leonardomso/rust-skills) as a
git submodule at [`.cursor/skills/rust-skills/`](.cursor/skills/rust-skills/). It
provides 265 focused Rust rules for AI-assisted coding and review.

After cloning the repository, initialize the submodule:

```powershell
git submodule update --init --recursive
```

In Cursor, the skill is available automatically. For Rust files, the rule in
[`.cursor/rules/rust-skills.mdc`](.cursor/rules/rust-skills.mdc) points the agent
to the skill index. Invoke it explicitly:

```text
/rust-skills review this function
```

The agent worker also references these guidelines in
[`settings/rules.md`](settings/rules.md) when generating Rust deliverables under
`out/`.

## Development

The project pins a stable Rust toolchain with `rustfmt` and `clippy` via
[`rust-toolchain.toml`](rust-toolchain.toml). Lint rules live in
[`Cargo.toml`](Cargo.toml) (`clippy::all` + `clippy::pedantic`).

Verify the toolchain is active:

```powershell
rustup show
```

Run all checks before committing (fmt, clippy, cargo test, and AoC agent
regression against `local/aoc-bank`):

```powershell
$env:POLZA_API_KEY = "..."   # only if not using provider.api_key or .env
.\scripts\check.ps1
.\scripts\check.ps1 -SkipAoc  # skip live agent eval when needed
```

```bash
export POLZA_API_KEY="..."   # only if not using provider.api_key or .env
./scripts/check.sh
./scripts/check.sh --skip-aoc  # skip live agent eval when needed
```

Individual commands:

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
.\scripts\aoc-regression.ps1
```

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
./scripts/aoc-regression.sh
```

Cursor/VS Code picks up [`.vscode/settings.json`](.vscode/settings.json) for
format-on-save and clippy diagnostics via rust-analyzer.

## Output format

The CLI prints exactly one JSON document to stdout:

```json
{
  "result": "final result text",
  "usage": {
    "iterations": 4,
    "prompt_tokens": 1234,
    "completion_tokens": 567,
    "total_tokens": 1801,
    "elapsed_ms": 42120
  },
  "stop_reason": "goal_reached",
  "logs": {
    "ai_log": "~/.agent-kuibyshev/logs/ai_usage.jsonl",
    "mcp_log": "~/.agent-kuibyshev/logs/mcp_usage.jsonl",
    "system_log": "~/.agent-kuibyshev/logs/agent.trace.log",
    "chat_log": null
  }
}
```

## Logging

By default the agent writes detailed logs under `~/.agent-kuibyshev/logs`:

| File | Content |
|------|---------|
| `agent.trace.log` | Technical `tracing` output (also mirrored to stderr) |
| `ai_usage.jsonl` | Structured AI completion events (when `enable_ai_log: true`) |
| `mcp_usage.jsonl` | Structured MCP tool events (when `enable_mcp_log: true`) |
| `chat_history.json` | Chat transcript pruned to the same `provider.history` budgets as the model window (when `enable_chat_history: true` or `--save-chat-history`) |

Override the base directory with any of:

- `AGENT_LOG_DIR` environment variable
- legacy `logging.output_dir` in config
- `logging.sink.path` for file sinks

Future sinks (for example database-backed storage) are configured via
`logging.sink.type`. The current release ships the `file` sink; `db` is reserved
for a later implementation.

Enable full chat history from the CLI:

```powershell
cargo run --bin agent_Kuibyshev -- run ... --save-chat-history
```

Example:

```yaml
logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: false
  sink:
    type: file
    # path: "./custom-logs"   # optional; defaults to ~/.agent-kuibyshev/logs
```

## Skills DSL

Current DSL form:

```text
skill "name" {
  policy: "string_policy"
  allowed_tools: ["home.read", "home.write", "home.run", "server.tool"]
}
```

The union of skill `allowed_tools` intersects with `access.tools.builtins` for
built-ins. Declared MCP tools are trusted automatically and are not filtered by
skills. Tool names must be qualified `server.tool` (bare names are rejected).

## Reliability notes

- Config is validated before start; unknown `access` fields are rejected.
- Provider and MCP calls use timeout controls.
- Provider retries on transport errors and `429/5xx`; cross-origin redirects and
  HTTP proxies are disabled.
- Agent exits deterministically on limits and emits `limit_reached`.
- AI and MCP logs are JSONL for auditing.
- Policy and sandbox decisions are logged as metadata (capability, allow/deny,
  exit/truncation); secrets and full env are not logged.

## Development

- Further improvement roadmap: [docs/FURTHER_FIXES.md](docs/FURTHER_FIXES.md)
- Linux namespace sandbox remote testing: [crates/sandbox-linux/TESTING.md](crates/sandbox-linux/TESTING.md)
- Orchestrator contract (access, sandbox, migration): [CONTRACT.md](CONTRACT.md)

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo +1.86.0 check --workspace
```

Platform suites (do not silently skip on the matching OS):

```powershell
cargo test -p sandbox-windows --test appcontainer
# On Linux (see crates/sandbox-linux/TESTING.md):
# cargo test -p sandbox-linux --test namespaces -- --test-threads=1
```
