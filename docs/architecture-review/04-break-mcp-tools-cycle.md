# 04 — Разорвать цикл mcp ↔ tools

**Status:** done  
**Severity:** P1  
**Area:** `src/tool_api.rs`, `src/mcp/mod.rs`, `src/tools/error.rs`, `src/tools/mod.rs`  
**Rust-skills:** `proj-mod-by-feature`, `err-custom-type`, `api-sealed-trait`, `anti-type-erasure`

## Problem

Концептуальный цикл модулей:

- `mcp::ToolExecutor` возвращает `tools::ToolError`
- `tools::ToolError` содержит `Mcp(mcp::McpError)`

Трейт исполнителя инструментов живёт в MCP-адаптере, хотя builtins тоже его реализуют. Инверсия зависимости.

## Evidence

- `src/mcp/mod.rs` — `use crate::tools::ToolError` + trait `ToolExecutor`
- `src/tools/error.rs` — `ToolError::Mcp(#[from] crate::mcp::McpError)`

## Acceptance

- [x] Нейтральный модуль (например `tool_api` или `tools::api`) владеет `ToolExecutor` + transport-neutral `ToolError`
- [x] `mcp` зависит от `tool_api`, не наоборот для трейта
- [x] `agent/loop` не импортирует `stdio_client` ради ошибок
- [x] `cargo` граф без циклов; тесты зелёные

## Suggested approach

1. Ввести `src/tool_api.rs` (или `src/tools/api.rs`) с trait + error.
2. `McpError` → `ToolError` через `From` на границе mcp.
3. Перенести re-export, обновить imports в agent/tools/mcp/main.
4. Опционально: sealed trait, если не хотим внешних impl.

## Resolution

- `src/tool_api.rs` owns `ToolExecutor`, domain errors, and transport-neutral `ToolError::External(ExternalToolError)`.
- `mcp` maps `McpError` → `ExternalToolError` / `ToolError` at the boundary; re-exports `ToolExecutor` for convenience.
- `tools` depends only on `tool_api` (no `mcp` import for the trait).
- `agent/loop` imports `tool_api::{ToolExecutor, ToolError}` — never `stdio_client` for errors.

## Blocks / related

- Упрощает [06](06-tool-descriptor-registry.md).
