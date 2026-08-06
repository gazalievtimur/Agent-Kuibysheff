# agent_Kuibysheff

[![CI](https://github.com/gybson63/Agent-Kuibysheff/actions/workflows/ci.yml/badge.svg)](https://github.com/gybson63/Agent-Kuibysheff/actions/workflows/ci.yml)

Minimal and reliable CLI agent worker in Rust.

## What it does

- Takes runtime config, an agent settings directory, a prompt, read-only input
  files, and an isolated home workspace.
- Runs an iterative agent loop against an OpenAI-compatible `/chat/completions` endpoint.
- Uses MCP servers over `stdio` or Streamable HTTP when the model requests tools.
- Can also invoke MCP tools as ordered Event-MCP middleware around context and response stages.
- Enforces hard stop limits: iterations, tokens, and max duration.
- Enforces an optional fail-closed `access` policy (tools, paths, `home.run`
  programs) and runs `home.run` inside an OS sandbox (Linux namespaces /
  Windows AppContainer) with no network.
- Produces a final JSON result with per-request token/cost accounting, exact
  decimal totals, and optional AI/MCP logs.

The CLI is a worker, not an orchestrator. It never applies generated files to a
target repository. See [CONTRACT.md](CONTRACT.md) for the stable interface an
external orchestrator must implement, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
for a high-level architectural overview.

For a multi-stage **1C development conveyor** (Jira/Confluence intake → analysis
→ coder → CFE packaging), see [workflows/1c-dev/README.md](workflows/1c-dev/README.md)
and `scripts/1c-dev-run.ps1`. To drive the same four stages from **VS Code via
ACP** (human switches agents; prepare/promote helper), see
[workflows/1c-dev/VSCODE.md](workflows/1c-dev/VSCODE.md).

For a **live Advent of Code** example that speaks ACP stdio from a Python
singleton (download puzzle → agent solve → submit → retry ≤5), see
[workflows/aoc-live/README.md](workflows/aoc-live/README.md).

## Inputs

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

Scaffold a new agent profile:

```text
agent_Kuibysheff help
agent_Kuibysheff init <agent-id> [--path DIR] [--force] [-i|--interactive]
```

Creates `./<agent-id>/` (or `--path`) with settings files and
`agent-config.example.yaml`. `--interactive` prompts for provider and limits.

Check that configured resources are reachable before a run:

```text
agent_Kuibysheff check --config <FILE> [--settings-dir <DIR>]
```

Probes the provider API key/HTTP endpoint, each MCP server, access paths and
programs, logging dir, optional settings files, and the OS sandbox when
`home.run` programs are configured. Exit code is non-zero if any probe fails.

### VS Code / ACP / external bridges

To use this agent from VS Code, a messenger bridge, or another ACP client, run
the ACP stdio server instead of one-shot `run`:

```text
agent_Kuibysheff acp \
  --config <FILE> \
  --settings-dir <DIR> \
  --home <DIR> \
  [--project-root <DIR>]
```

**Stream rules:** `stdin`/`stdout` are ACP JSON-RPC only; put diagnostics on a
separate `stderr` pipe and drain it. Prefer one long-lived process per config.
Each `session/prompt` is independent — the bridge owns chat/mail thread history.
Messenger and email credentials stay in the external app, not in this binary.

VS Code’s Agent Host speaks AHP to the UI; Kuibysheff is the ACP backend.
For **1C products**, open the product folder as the workspace, scaffold
`.kuibysheff/` (`scripts/1c-dev-scaffold-project.ps1` or the
[`extensions/vscode`](extensions/vscode/) sidebar), and use the ACP preset in
[`workflows/1c-dev/vscode/`](workflows/1c-dev/vscode/). Session `cwd` /
`--project-root` point at that folder; agent MCP/workspace settings live in
`.kuibysheff/agents/*/agent-config.yaml`. Guide:
[workflows/1c-dev/VSCODE.md](workflows/1c-dev/VSCODE.md).
Extension README: [extensions/vscode/README.md](extensions/vscode/README.md).

See [CONTRACT.md](CONTRACT.md#acp-ide-messengers-mail-bridges) for the full ACP
bridge contract.

## Releases

Prebuilt binaries for Windows and Linux are published on
[GitHub Releases](https://github.com/gybson63/Agent-Kuibysheff/releases)
when a version tag is pushed.

| Platform | Archive |
| --- | --- |
| Windows x86_64 | `agent_Kuibysheff-vX.Y.Z-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `agent_Kuibysheff-vX.Y.Z-x86_64-unknown-linux-gnu.zip` |

Each archive contains the `agent_Kuibysheff` binary. A matching
`.zip.sha256` checksum file is attached to the release.

PowerShell (Windows):

```powershell
# After downloading and extracting the zip:
.\agent_Kuibysheff-v0.1.0-x86_64-pc-windows-msvc.exe --help
```

Bash (Linux):

```bash
# After downloading and extracting the zip:
chmod +x ./agent_Kuibysheff-v0.1.0-x86_64-unknown-linux-gnu
./agent_Kuibysheff-v0.1.0-x86_64-unknown-linux-gnu --help
```

To cut a release from a commit on `main`:

```bash
# Keep Cargo.toml version in sync with the tag when bumping.
git tag v0.1.0
git push origin v0.1.0
```

The [Release](.github/workflows/release.yml) workflow builds
`--release --locked --bin agent_Kuibysheff` on `windows-latest` and
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
cargo run --bin agent_Kuibysheff -- run `
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
- JSONL event logs and tracing output in `~/.agent-kuibysheff/logs` by default

Runtime limit overrides remain available:

```powershell
cargo run --bin agent_Kuibysheff -- run <required arguments> `
  --max-iterations 20 --max-tokens 25000 --max-duration-sec 180
```

Model context-window pruning is configured under `provider.history` (defaults
`max_tail_messages: 30`, `max_chars: 200000`). Raise these for long-context
models; they are independent of `limits.max_tokens` (the run stop budget).

Exact cost accounting is configured under `billing`; `limits.max_cost` or
`--max-cost USD:1.00` adds a fail-soft monetary stop budget. Provider-reported
cost, an optional dedicated MCP calculator, and a versioned local catalog can be
ordered as pricing sources. See [docs/BILLING.md](docs/BILLING.md).

See [`agent-config.example.yaml`](agent-config.example.yaml) for runtime config
(including the required `access` policy),
[`settings/`](settings/) for the settings layout,
[`test-agents/`](test-agents/) for specialized test agent profiles, and
[`prompt-examples.md`](prompt-examples.md) for ready-to-use `--prompt` templates.

## Access policy and sandbox

`access` is required. Prefer fail-closed tools, path grants, and program
aliases in production. Permissive FS is only via explicit
`access: { mode: legacy }` (`home.run` stays hidden). Details and migration:
[CONTRACT.md](CONTRACT.md#access-policy-fail-closed).

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

### Pre-commit CI gate

Commits are gated by the same checks that CI runs first:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo +nightly miri test -p sandbox-linux --lib` (Linux only, when nightly is installed)

Enable the git hook once per clone:

```powershell
.\scripts\install-git-hooks.ps1
```

```bash
./scripts/install-git-hooks.sh
```

That sets `core.hooksPath=.githooks`. Cursor also blocks `git commit --no-verify`
and commits when hooks are not installed (see [`.cursor/hooks.json`](.cursor/hooks.json)).

Run the gate manually:

```powershell
.\scripts\pre-commit-gate.ps1
.\scripts\pre-commit-gate.ps1 -SkipMiri   # Windows / no nightly
```

```bash
./scripts/pre-commit-gate.sh
./scripts/pre-commit-gate.sh --skip-miri
```

Emergency bypass (local only): `SKIP_PRECOMMIT=1`.

### Full local quality gate

Run all checks before pushing (fmt, clippy, cargo test, and AoC agent
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
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.\scripts\aoc-regression.ps1
```

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/aoc-regression.sh
```

Cursor/VS Code picks up [`.vscode/settings.json`](.vscode/settings.json) for
format-on-save and clippy diagnostics via rust-analyzer.

## Output format

The CLI prints exactly one JSON document to stdout:

```json
{
  "run_id": "run-...",
  "result": "final result text",
  "usage": {
    "iterations": 4,
    "prompt_tokens": 1234,
    "completion_tokens": 567,
    "total_tokens": 1801,
    "elapsed_ms": 42120,
    "cost": {
      "status": "complete",
      "known_total": { "amount": "0.004812", "currency": "USD" },
      "priced_requests": 4,
      "unpriced_requests": 0,
      "budget_status": "not_configured",
      "requests": []
    }
  },
  "stop_reason": "goal_reached",
  "logs": {
    "ai_log": "~/.agent-kuibysheff/logs/ai_usage.jsonl",
    "mcp_log": "~/.agent-kuibysheff/logs/mcp_usage.jsonl",
    "system_log": "~/.agent-kuibysheff/logs/agent.trace.log",
    "chat_log": null
  }
}
```

Process exit for `run`: `0` when `stop_reason` is `goal_reached` or
`limit_reached`; non-zero when `stop_reason` is `error` (JSON is still printed
on stdout). Management commands (`init`, `check`) keep their own exit semantics
and do not emit `RunOutput`.

## Event-MCP middleware

Event-MCP connects MCP servers to the agent's information flow, not only to
model-selected tool calls. It uses the standard MCP `tools/call` method: an MCP
server exposes a handler as an ordinary tool, and the agent invokes that tool at
an explicitly configured pipeline stage. No custom transport or JSON-RPC method
is required.

Several handlers may subscribe to the same event. They run sequentially in the
order listed in the configuration, regardless of MCP server connection order.
Each handler receives the last valid payload, so a transformation made by one
handler becomes the input of the next:

```yaml
mcp:
  - name: security
    transport: stdio
    command: "security-mcp"
  - name: context
    transport: http
    url: "https://mcp.example.com/mcp"

event_mcp:
  events:
    context.before_model:
      handlers:
        - id: redact-secrets
          target: security.redact
          timeout_ms: 3000
          on_error: abort
        - id: compact-history
          target: context.compact
          timeout_ms: 5000
          on_error: continue
```

The MVP provides three transformation stages:

- `context.before_model` receives a snapshot of chat messages immediately
  before each provider request. The transformed snapshot is sent to the model,
  while canonical chat history remains unchanged.
- `model.after_response` receives provider text before audit logging and JSON
  directive parsing. It can validate or repair model output.
- `run.before_output` receives the final result before it is emitted to ACP or
  placed in `RunOutput`. It can validate or format the user-facing response.

Handlers return an MCP `CallToolResult`. The preferred result is
`structuredContent` containing one of:

```json
{ "action": "pass" }
{ "action": "replace", "payload": {} }
{ "action": "reject", "reason": "validation failed" }
```

`pass` preserves the current payload, `replace` passes the supplied payload to
the next handler, and `reject` stops the chain and fails the run. A timeout,
transport failure, or malformed result follows the handler's `on_error` policy:
`continue` keeps the last valid payload, while `abort` fails the run.
Cancellation always stops the chain.

Bindings are validated after MCP tool discovery. Configuring a handler is an
explicit trust grant to send that event payload to its MCP server, but it does
not add the tool to the model's skills/tool allowlist. Event audit records
contain stage, target, sizes, duration, and outcome—not prompt or response
bodies.

The complete versioned envelope, payload schemas, security boundary, and future
`run.input` OCR/vision extension are documented in
[docs/EVENT_MCP.md](docs/EVENT_MCP.md).

## Logging

By default the agent writes detailed logs under `~/.agent-kuibysheff/logs`:

| File | Content |
|------|---------|
| `agent.trace.log` | Technical `tracing` output (also mirrored to stderr) |
| `ai_usage.jsonl` | Structured AI completion, provider-attempt, token, and cost events (when `enable_ai_log: true`) |
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
cargo run --bin agent_Kuibysheff -- run ... --save-chat-history
```

Example:

```yaml
logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: false
  sink:
    type: file
    # path: "./custom-logs"   # optional; defaults to ~/.agent-kuibysheff/logs
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
built-ins, and with discovered MCP tools for MCP. Both classes must appear in
`skills.allowed_tools` to be advertised or callable. Skill `policy` strings are
prompt-only; the runtime enforces only qualified `allowed_tools` names. Tool
names must be qualified `server.tool` (bare names are rejected). See
[CONTRACT.md](CONTRACT.md#access-policy-fail-closed).

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
