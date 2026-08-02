# 03 — Audit-log failure не маскирует tool side effect

**Status:** done  
**Severity:** P0  
**Area:** `src/mcp/stdio_client.rs`, `src/agent/loop.rs`, `src/logging/`  
**Rust-skills:** `obs-error-chain`, `err-result-over-panic`, `anti-empty-catch`

## Problem

Если MCP `tools/call` успешен, а последующая запись в MCP/AI audit log падает, ошибка может вернуться модели как tool failure. Side effect уже произошёл, но агент «видит» отказ → повторные вызовы, рассинхрон состояния.

## Evidence

- MCP call path логирует arguments/results после выполнения
- Agent loop трактует `ToolError` как неуспех инструмента для модели

## Acceptance

- [x] Успешный tool result доставляется модели даже при сбое audit sink
- [x] Сбой логирования пишется в `tracing` / system log отдельно (warn/error)
- [x] Опционально: метрика/флаг в diagnostics «audit_write_failed»
- [x] Тест: mock sink fail после ok call → tool result Ok, log warn

## Suggested approach

1. Разделить `execute` и `audit`: audit ошибки не мапятся в `ToolError` для caller.
2. Единая политика: какие log failures fatal (только старт?), какие soft.
3. Не глотать ошибки молча — `obs-error-chain`, один раз на границе.

## Notes

Связано с общим вопросом «когда logging fatal» (FURTHER_FIXES / observability).

### Policy (implemented)

| Phase | Failure | Behavior |
|-------|---------|----------|
| Startup (`Loggers::from_config`, `mcp_server_initialized`) | sink open / init audit | **fatal** |
| Runtime after successful LLM/tool call | AI/MCP audit `write_event` | **soft** (`tracing` warn; continue) |

`Loggers::audit_write_failed()` / `run_summary.audit_write_failed` records soft write failures via `TrackingEventSink`.
