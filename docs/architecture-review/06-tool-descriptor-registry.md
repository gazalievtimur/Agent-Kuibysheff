# 06 — Единый ToolDescriptor registry

**Status:** in progress  
**Severity:** P1  
**Area:** `src/access/`, `src/tools/`, `src/prompt.rs`, `src/skills/`  
**Rust-skills:** `type-no-stringly`, `api-parse-dont-validate`, `proj-mod-by-feature`  
**Branch:** `feat/tool-descriptor-registry`

## Problem

Добавление builtin требует правок в нескольких местах:

| Место | Что хранит |
|-------|------------|
| `access` | `KNOWN_BUILTINS` |
| `tools/mod.rs` | routing `home` / `local_tools` |
| `fs_home` / `local_tools` | decode args + handlers |
| `prompt.rs` | схемы/примеры для модели |
| `skills.dsl` | allowlist |

Легко получить дрейф имён/схем/policy.

## Acceptance

- [x] Один `ToolDescriptor` (qualified name, description, JSON schema, policy tags, handler id)
- [x] Advertising, policy compile и dispatch читают один registry
- [x] Добавление tool = одна регистрация + impl handler
- [x] Тест: registry names == advertised == policy-known set

## Suggested approach

1. После [04](04-break-mcp-tools-cycle.md) завести `tools::registry`.
2. Builtins регистрируются статически (`inventory` / const slice / once_cell).
3. MCP tools добавляются динамически после `tools/list` (name + optional schema; сейчас schema discard — рассмотреть сохранение).
4. `prompt::build_runtime_rules` генерирует фрагмент из descriptors.

## Depends on

- Желательно после [04](04-break-mcp-tools-cycle.md).

## Resolution (this PR)

- `src/tools/registry.rs` owns `ToolDescriptor` + static `BUILTINS`.
- `access` derives known/legacy sets and reserved MCP server names from the registry.
- `prompt::build_runtime_rules` iterates descriptors (special-cases `home.run` / workspace read).
- `CompositeToolExecutor` routes via `handler_for_server` and advertises registry names.
- Deferred: MCP dynamic descriptors with retained schemas; skills.dsl validation against registry.
