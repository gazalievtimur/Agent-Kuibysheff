# Output format

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

For cost fields and pricing sources, see [BILLING.md](BILLING.md).
