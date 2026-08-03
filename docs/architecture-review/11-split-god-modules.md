# 11 — Разбить god-modules

**Status:** in progress (`agent/loop` done; MCP/config/access remain)  
**Severity:** P2  
**Area:** крупные файлы в `src/`  
**Rust-skills:** `proj-mod-by-feature`, `proj-flat-small`, `anti-over-abstraction`

## Problem

Несколько файлов >900 LOC смешивают ответственности — сложнее review и точечные фиксы P0/P1.

| File | ~LOC | Смешение | Status |
|------|------|----------|--------|
| `mcp/http_client.rs` | 1214 | transport · session · SSE | open |
| `mcp/oauth.rs` | 1103 | auth · callback · persistence | open |
| `config.rs` | 1057 | DTO · validate · CLI overrides | open |
| `agent/loop/` | — | parse · history · engine | **done** (`directive` + `history` + `engine`) |
| `access/mod.rs` | 936 | types · compile · tests | open |

## Acceptance

- [x] Каждый split — по feature (не «types.rs / impls.rs» ради файла) — for `agent/loop`
- [x] Публичные reexport сохраняют совместимость внутри crate — for `agent/loop`
- [x] Нет роста abstraction (generics/dyn) без нужды — for `agent/loop`
- [x] Clippy/test зелёные после каждого под-PR — for `agent/loop`
- [ ] Remaining subsystems (http_client, oauth, config, access)

## Suggested approach (порядок)

1. ~~`agent/loop` → `directive` + `history` + `engine`~~ (done; помогает [07](07-hard-deadline-cancellation.md))
2. `mcp/http_client` → session / sse / client
3. `oauth` → flow / store / callback
4. `config` — после [05](05-break-config-access-cycle.md)
5. `access` — types vs compile vs paths (paths уже отдельно)

## Notes

Делать **отдельными PR на подсистему**, не одним мега-diff.
