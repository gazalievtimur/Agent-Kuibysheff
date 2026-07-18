# agent_Kuibyshev

Minimal and reliable CLI agent worker in Rust.

## What it does

- Takes runtime config, an agent settings directory, a prompt, read-only input
  files, and an isolated home workspace.
- Runs an iterative agent loop against an OpenAI-compatible `/chat/completions` endpoint.
- Uses MCP servers over `stdio` when the model requests tools.
- Enforces hard stop limits: iterations, tokens, and max duration.
- Provides sandboxed `home.list`, `home.read`, `home.write`, and `home.run` tools.
- Produces a final JSON result with usage stats and optional AI/MCP logs.

The CLI is a worker, not an orchestrator. It never applies generated files to a
target repository. See [CONTRACT.md](CONTRACT.md) for the stable interface an
external orchestrator must implement.

## Inputs

```text
--config <FILE>          provider / MCP / limits / logging
--settings-dir <DIR>     master_prompt.md / skills.dsl / optional rules.md
--prompt <TEXT>          task for this run
--home <DIR>             sandboxed writable workspace
--files <PATH>...        optional UTF-8 read-only context files
```

## Quick start

Prerequisites:
- Rust toolchain (`cargo`)

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
cargo run -- `
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
cargo run -- <required arguments> `
  --max-iterations 20 --max-tokens 25000 --max-duration-sec 180
```

See [`agent-config.example.yaml`](agent-config.example.yaml) for runtime config,
[`settings/`](settings/) for the settings layout,
[`test-agents/`](test-agents/) for specialized test agent profiles, and
[`prompt-examples.md`](prompt-examples.md) for ready-to-use `--prompt` templates.

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

Individual commands:

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
.\scripts\aoc-regression.ps1
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
| `chat_history.json` | Full unpruned chat transcript (when `enable_chat_history: true` or `--save-chat-history`) |

Override the base directory with any of:

- `AGENT_LOG_DIR` environment variable
- legacy `logging.output_dir` in config
- `logging.sink.path` for file sinks

Future sinks (for example database-backed storage) are configured via
`logging.sink.type`. The current release ships the `file` sink; `db` is reserved
for a later implementation.

Enable full chat history from the CLI:

```powershell
cargo run -- ... --save-chat-history
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

The union of `allowed_tools` is enforced at runtime. Qualified `server.tool`
names are recommended; bare tool names remain supported.

## Reliability notes

- Config is validated before start.
- Provider and MCP calls use timeout controls.
- Provider retries on transport errors and `429/5xx`.
- Agent exits deterministically on limits and emits `limit_reached`.
- AI and MCP logs are JSONL for auditing.

## Development

- Further improvement roadmap: [docs/FURTHER_FIXES.md](docs/FURTHER_FIXES.md)
