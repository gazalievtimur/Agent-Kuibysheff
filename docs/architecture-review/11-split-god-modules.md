# 11 — Разбить god-modules

**Status:** in progress (`agent/loop` split done; `run_inner` tracked in [17](17-decompose-engine-run-inner.md); MCP/config/access/billing remain)  
**Severity:** P2  
**Area:** крупные файлы в `src/`  
**Rust-skills:** `proj-mod-by-feature`, `proj-flat-small`, `anti-over-abstraction`

## Problem

Несколько файлов >900 LOC смешивают ответственности — сложнее review и точечные фиксы P0/P1.

Sizes below are non-blank line counts on 2026-09-29. `config` moved from `config.rs` to `config/mod.rs` and grew.

| File | ~LOC | Смешение | Status |
|------|------|----------|--------|
| `config/mod.rs` | 1570 | DTO · validate · CLI overrides · MCP wire serde | open |
| `agent/loop/engine.rs` | 1450 | `run_inner` ~610 lines; tests in the same file | open — function split is [17](17-decompose-engine-run-inner.md) |
| `mcp/stdio_client.rs` | 1258 | process · registry · `ToolExecutor` · sandbox env | open |
| `mcp/http_client.rs` | 1188 | transport · session · SSE | open |
| `billing/mod.rs` | 1144 | money · resolvers · catalog · tracker | open |
| `mcp/oauth.rs` | 1095 | auth · callback · persistence | open |
| `provider/openai_compat.rs` | 1090 | HTTP · retries · cost headers | open |
| `access/mod.rs` | 1030 | types · compile · tests | open |
| `agent/loop/` (`directive`, `history`) | — | parse · history | **done** |

## Acceptance

- [x] Каждый split — по feature (не «types.rs / impls.rs» ради файла) — for `agent/loop`
- [x] Публичные reexport сохраняют совместимость внутри crate — for `agent/loop`
- [x] Нет роста abstraction (generics/dyn) без нужды — for `agent/loop`
- [x] Clippy/test зелёные после каждого под-PR — for `agent/loop`
- [ ] Remaining subsystems (`config/mod.rs`, `stdio_client`, `http_client`, `billing`, `oauth`, `openai_compat`, `access`)

## Suggested approach (порядок)

1. ~~`agent/loop` → `directive` + `history` + `engine`~~ (done; помогает [07](07-hard-deadline-cancellation.md)). Дальше резать `run_inner` — [17](17-decompose-engine-run-inner.md), не новый файл ради файла.
2. `mcp/stdio_client` → process / registry / executor
3. `mcp/http_client` → session / sse / client
4. `oauth` → flow / store / callback
5. `billing` → money / resolvers / catalog (после [14](14-break-remaining-module-cycles.md), чтобы не тащить цикл с `limits`)
6. `provider/openai_compat` → request / retry / accounting
7. `config` — [05](05-break-config-access-cycle.md) done; DTO vs load/validate можно отделять. Logging DTO, если уедут в [14](14-break-remaining-module-cycles.md), не резать здесь второй раз.
8. `access` — types vs compile vs paths (paths уже отдельно)

## Notes

Делать **отдельными PR на подсистему**, не одним мега-diff.
