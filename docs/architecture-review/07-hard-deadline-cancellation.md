# 07 — Hard deadline + CancellationToken

**Status:** done  
**Severity:** P1  
**Area:** `src/agent/loop.rs`, `src/agent/run_cancel.rs`, `src/provider/`, `src/mcp/`, `src/sandbox/`, `src/main.rs`  
**Rust-skills:** `async-cancellation-token`, `async-cancel-safety`, `async-select-racing`  
**Branch:** `feat/hard-deadline-cancellation`

## Problem

`limits.max_duration_sec` проверяется между итерациями. In-flight provider HTTP, MCP call или sandbox run могут уйти далеко за бюджет. Нет кооперативной отмены.

## Evidence

- Loop checks duration after complete / before next iter
- Нет `CancellationToken` в `AgentRunRequest` / MCP actor / provider client
- Частично foreshadowed в `docs/FURTHER_FIXES.md` §2.3

## Acceptance

- [x] Token из composition root → engine → provider/MCP/sandbox где возможно
- [x] По истечении deadline: прекращение новых итераций + отмена in-flight где cancel-safe
- [x] Новый или существующий `stop_reason` отражает timeout/cancel (согласовать с CONTRACT)
- [x] Тест: mock slow provider + короткий deadline → выход без бесконечного wait

## Implementation notes

- `RunCancel` (`tokio_util::sync::CancellationToken` + armed deadline) создаётся в `main`, шарится в `HomeFs` / MCP actors / `AgentRunRequest`.
- Engine arms deadline при старте `run` (совпадает с `RunMetrics` wall clock).
- Agent loop: `select!` вокруг `model.complete` и `tools.call_tool`; drop future отменяет in-flight HTTP.
- MCP: `select!` на enqueue/wait и в actor loop; при cancel — `McpError::Cancelled`.
- Sandbox: `home.run` clamps `timeout_ms` к remaining run budget.
- `stop_reason: limit_reached` + `max_duration_sec` (без нового enum variant).
- Cancel-safety: частичные tool side effects возможны после cancel.

## Related

- [02](02-mcp-stdio-child-shutdown.md) — shutdown на cancel
- FURTHER_FIXES §2.3
