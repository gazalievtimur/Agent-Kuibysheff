# agent_Kuibyshev

Minimal and reliable CLI agent worker in Rust.

## What it does

- Takes runtime config, an agent settings directory, a prompt, read-only input
  files, and an isolated home workspace.
- Runs an iterative agent loop against an OpenAI-compatible `/chat/completions` endpoint.
- Uses MCP servers over `stdio` when the model requests tools.
- Enforces hard stop limits: iterations, tokens, and max duration.
- Provides sandboxed `home.list`, `home.read`, and `home.write` tools.
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

Repository includes a local MCP stdio server: `mcp-server.js`.

Prerequisites:
- Rust toolchain (`cargo`)
- Node.js in PATH (`node --version`)

PowerShell commands:

```powershell
$env:OPENAI_API_KEY = "your_api_key"
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
- Local MCP tools from `mcp-server.js` (`search_docs`, `read_file`)
- Built-in filesystem tools restricted to `./demo-home`
- Agent behavior from the `./settings` directory
- JSONL logs in `./logs`

Runtime limit overrides remain available:

```powershell
cargo run -- <required arguments> `
  --max-iterations 20 --max-tokens 25000 --max-duration-sec 180
```

See [`agent-config.example.yaml`](agent-config.example.yaml) for runtime config,
[`settings/`](settings/) for the settings layout, and
[`prompt-examples.md`](prompt-examples.md) for ready-to-use `--prompt` templates.

## Development

The project pins a stable Rust toolchain with `rustfmt` and `clippy` via
[`rust-toolchain.toml`](rust-toolchain.toml). Lint rules live in
[`Cargo.toml`](Cargo.toml) (`clippy::all` + `clippy::pedantic`).

Verify the toolchain is active:

```powershell
rustup show
```

Run all checks before committing:

```powershell
.\scripts\check.ps1
```

Individual commands:

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
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
    "ai_log": "logs/ai_usage.jsonl",
    "mcp_log": "logs/mcp_usage.jsonl"
  }
}
```

## Skills DSL

Current DSL form:

```text
skill "name" {
  policy: "string_policy"
  allowed_tools: ["home.read", "home.write", "server.tool"]
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
