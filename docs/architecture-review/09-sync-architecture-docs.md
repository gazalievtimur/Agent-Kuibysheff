# 09 — Синхронизировать ARCHITECTURE.md с CONTRACT/кодом

**Status:** done  
**Severity:** P1  
**Area:** `docs/ARCHITECTURE.md`, `CONTRACT.md`  
**Rust-skills:** `doc-all-public`, `doc-link-types`

## Problem

В `docs/ARCHITECTURE.md` указано, что MCP tools «currently trusted, no skills intersection».  
В `CONTRACT.md` и в коде (`EffectiveToolPolicy::compile`):

```text
effective_mcp = discovered_mcp_tools ∩ skills.allowed_tools
```

Документ архитектуры вводит в заблуждение при threat modeling.

## Evidence

- `docs/ARCHITECTURE.md` — секция Access policy / effective_mcp
- `CONTRACT.md` — Access policy formula
- `src/access/mod.rs` — doc comment на `compile`

## Acceptance

- [x] ARCHITECTURE.md формула совпадает с CONTRACT и кодом
- [x] Упомянуты: builtins intersection, MCP×skills, prompt-only `policy` string
- [x] Краткая ссылка на этот backlog / P0 MCP framing, если уместно
- [x] Нет противоречий README ↔ ARCHITECTURE ↔ CONTRACT по access

## Suggested approach

1. Скопировать каноническую формулу из CONTRACT.
2. Пройтись grep по «trusted» / «no skills» в docs.
3. Один PR только docs — без code churn.

## Notes

Чисто документальный пункт; можно закрыть быстро отдельно от P0 кода.
