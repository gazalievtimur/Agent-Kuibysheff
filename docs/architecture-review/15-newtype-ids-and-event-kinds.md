# 15 — Newtypes for ids and audit event kinds

**Status:** open
**Severity:** P2
**Area:** `src/cli/`, `src/app.rs`, `src/output.rs`, `src/mcp/http_client.rs`, `src/logging/sink.rs`, `src/tool_api.rs`, `src/agent/loop/engine.rs`
**Rust-skills:** `type-newtype-ids`, `type-newtype-validated`, `api-newtype-safety`, `type-no-stringly`
**Review:** [review-2026-09.md](review-2026-09.md)

## Problem

Run, agent, and session identities are bare `String`. Audit event names are free `&str`, so a typo is a silent new event kind. `ToolExecutor::call_tool` takes `server` and `tool` as `&str` even though `QualifiedTool` already validates that pair.

The `call_tool` signature is on the stable library surface (`tool_api`, re-exported from `prelude`). Changing it is a minor-version bump and a `CONTRACT.md` update, not a drive-by edit.

## Evidence

| Value | Where |
|-------|--------|
| `agent: String`, `run_id: Option<String>` | `src/cli/mod.rs` (identity and run args) |
| `agent_id`, `run_id`, `prompt` on `AgentPromptArgs` | `src/app.rs:49–65` |
| `run_id: String` on `RunOutput` | `src/output.rs` |
| `session_id: Option<String>` | `src/mcp/http_client.rs` |
| length check `1..=128` inline | `src/app.rs` (`run_agent_prompt`) |

```rust
// src/logging/sink.rs
async fn write_event(&self, event_type: &str, payload: Value) -> Result<(), LoggingError>;
```

Call sites pass `"tool_call_started"`, `"tool_call_failed"`, `"tool_call_finished"`, `"ai_completion"`, `"directive_parse_failed"`, `"run_summary"`, and similar literals from `src/agent/loop/engine.rs`.

```rust
// src/tool_api.rs
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ToolError>;
    fn available_tools(&self) -> Vec<String>;
}
```

## Acceptance

- [ ] `RunId`, `AgentId`, and `SessionId` newtypes. `RunId::new` owns the `1..=128` check (reject empty and over-long). Invalid values never reach `AgentEngine`.
- [ ] `AuditEventKind` enum with `as_str()`; `EventSink::write_event` and `AgentEngine::log_tool_event` take the enum. JSONL field name stays the same string.
- [ ] `ToolExecutor::call_tool` takes `&QualifiedTool` (or an equivalent parsed type). `available_tools` may stay `Vec<String>` at the wire edge if MCP discovery still yields strings, but the engine passes a parsed tool.
- [ ] Crate version bump and `CONTRACT.md` note for the `tool_api` change.
- [ ] Tests cover rejected `RunId` and a compile-fail or match exhaustiveness check that a new audit kind cannot be a raw string.

## Suggested approach

1. Land [14](14-break-remaining-module-cycles.md) first so `QualifiedTool` and the registry have a stable module.
2. Add the newtypes next to their domains (`output` or `agent` for `RunId`, `project_paths` for `AgentId`, protocol modules for `SessionId`). Implement `Display` and `AsRef<str>`.
3. Introduce `AuditEventKind` in `logging` and switch engine literals in one commit.
4. Change `call_tool` last, with the version bump, so ACP/A2A and tests update together.

## Related

- [06](06-tool-descriptor-registry.md), [14](14-break-remaining-module-cycles.md).
- [18](18-app-composition-root-hygiene.md) — `AgentPromptArgs` is the bag these ids currently sit on.
