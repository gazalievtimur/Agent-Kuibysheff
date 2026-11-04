# Logging

By default the agent writes detailed logs under `~/.agent-kuibysheff/logs`:

| File | Content |
|------|---------|
| `agent.trace.log` | Technical `tracing` output (also mirrored to stderr) |
| `ai_usage.jsonl` | Structured AI completion, provider-attempt, token, and cost events (when `enable_ai_log: true`) |
| `mcp_usage.jsonl` | Structured MCP tool events (when `enable_mcp_log: true`) |
| `chat_history.json` | Full unpruned chat transcript (when `enable_chat_history: true` or `--save-chat-history`). Used by `--continue-from` / Studio **Продолжить** to resume a run |

Override the base directory with any of:

- `AGENT_LOG_DIR` environment variable
- legacy `logging.output_dir` in config
- `logging.sink.path` for file sinks

Future sinks (for example database-backed storage) are configured via
`logging.sink.type`. The current release ships the `file` sink; `db` is reserved
for a later implementation.

## Audit redaction

AI/MCP JSONL sinks apply redaction before write (chat history and `agent.trace.log` are unchanged):

- Sensitive object keys (case-insensitive), including `api_key`, `authorization`, `password`, `secret`, `token`, `access_token`, `refresh_token`, `client_secret`, `private_key`, `cookie`, `set-cookie`, `bearer`, plus any `extra_sensitive_keys`
- String leaves longer than `max_string_chars` are truncated with a `…[truncated]` suffix

Defaults: `enabled: true`, `max_string_chars: 4096`. Set `enabled: false` for legacy full payloads.

Enable full chat history from the CLI:

```powershell
cargo run --bin kbshff -- run ... --save-chat-history
```

Resume from a previously saved transcript (prompt optional):

```powershell
cargo run --bin kbshff -- run `
  --project-root . `
  --agent demo `
  --continue-from .kuibysheff/homes/demo/logs/chat_history.json `
  --save-chat-history
```

The path must stay under `--project-root` and outside `.kuibysheff/protected/`.
Studio Test run writes the same file under `{home}/logs/` via `AGENT_LOG_DIR`.

Example:

```yaml
logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: false
  audit_redaction:
    enabled: true
    max_string_chars: 4096
    # extra_sensitive_keys: ["session_token"]
  sink:
    type: file
    # path: "./custom-logs"   # optional; defaults to ~/.agent-kuibysheff/logs
```
