# 10 — Curated public API (lib.rs)

**Status:** done  
**Severity:** P2  
**Area:** `src/lib.rs`, модули crate  
**Rust-skills:** `proj-pub-crate-internal`, `proj-pub-use-reexport`, `api-non-exhaustive`, `doc-all-public`

## Problem

`lib.rs` делает `pub mod` почти всё: cli, commands, oauth, concrete MCP transports, logging sinks, sandbox mocks. Любая внутренняя деталь становится semver-обязательством, если crate потребляют как библиотеку.

## Evidence

- `src/lib.rs` — полный список `pub mod …`
- Binary composition root в `main.rs` тянет глубокие пути

## Acceptance

- [x] Публичный facade: run types, stable config, extension traits, sandbox abstractions
- [x] Concrete adapters / parsers / clap structs — `pub(crate)` по умолчанию
- [x] Binary и integration tests компилируются
- [x] Краткий `docs` / module rustdoc: что считается stable

## Suggested approach

1. Инвентаризация внешних потребителей (если только binary — смелее `pub(crate)`).
2. `pub use` reexport узкого prelude.
3. Поэтапно: сначала mcp oauth/sse, потом cli/commands.

## Notes

- Composition root перенесён в `src/app.rs`; `main.rs` только `try_run_helper` + `app::run()`.
- `pub(crate)`: `cli`, `commands`, `context`, `prompt`, `settings`, `skills`, MCP transports (oauth/sse/http/stdio), `provider::openai_compat`, logging sink adapters, `MockBackend`.
- Stable reexports: `mcp::{McpRegistry, BearerChallenge}`, `prelude`.

## Related

- Снижает шум после [04](04-break-mcp-tools-cycle.md) / [05](05-break-config-access-cycle.md).
