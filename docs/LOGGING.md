# Logging

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
