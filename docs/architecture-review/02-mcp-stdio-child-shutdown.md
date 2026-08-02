# 02 — Явный shutdown stdio MCP child

**Status:** done  
**Severity:** P0  
**Area:** `src/mcp/stdio_client.rs`  
**Rust-skills:** `async-cancellation-token`, `err-result-over-panic`, `obs-structured-fields`

## Problem

`LiveClient::shutdown` обрабатывает HTTP-сессию; для stdio `Child` нет явного terminate/wait. Drop Tokio `Child` не гарантирует завершение процесса → риск зомби MCP-серверов и удержания портов/файлов.

## Evidence

- `LiveClient` держит `Child`, но shutdown path покрывает HTTP
- Stdio actor закрывает канал, но не дожидается exit code процесса

## Acceptance

- [x] Shutdown sequence: close stdin → wait с timeout → `kill` при просрочке → `wait` после kill
- [x] Exit code / signal логируются в tracing (без секретов env)
- [x] Unit/integration test: fixture-процесс реально завершается
- [x] Drop/`Drop` path либо вызывает тот же shutdown, либо документированно best-effort + warn

## Suggested approach

1. Вынести `shutdown_stdio_child(child, timeout)` в helper.
2. Вызывать из registry drop / explicit disconnect / run end.
3. Связать с пунктом 07 (CancellationToken), но не блокировать этот фикс на нём.

## Depends on

- Желательно после или параллельно с [01](01-mcp-stdio-ndjson-framing.md) (тот же файл).

## Notes

- `McpRegistry::shutdown` закрывает actor channel и `await`-ит join (явный путь).
- `Drop` для `McpClientHandle` / `McpStdioClient` — best-effort: warn + `kill_on_drop(true)`.
- Полный cooperative cancel через `CancellationToken` остаётся в [07](07-hard-deadline-cancellation.md).
