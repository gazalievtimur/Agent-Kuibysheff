# Architecture review backlog

> Engineering backlog (not product documentation). Status notes may be in
> Russian or English.

Индекс пунктов из архитектурных оценок `agent_Kuibysheff`
(2026-08-02 и [2026-09-29](review-2026-09.md)).
Каждый пункт — отдельный файл; работаем по одному.

| ID | Sev | Файл | Тема | Status |
|----|-----|------|------|--------|
| 01 | P0 | [01-mcp-stdio-ndjson-framing.md](01-mcp-stdio-ndjson-framing.md) | MCP stdio: NDJSON вместо Content-Length | done |
| 02 | P0 | [02-mcp-stdio-child-shutdown.md](02-mcp-stdio-child-shutdown.md) | Явный shutdown stdio child | done |
| 03 | P0 | [03-audit-log-vs-tool-side-effect.md](03-audit-log-vs-tool-side-effect.md) | Audit-log не маскирует side effect | done |
| 04 | P1 | [04-break-mcp-tools-cycle.md](04-break-mcp-tools-cycle.md) | Разорвать цикл mcp ↔ tools | done |
| 05 | P1 | [05-break-config-access-cycle.md](05-break-config-access-cycle.md) | Разорвать цикл config ↔ access | done |
| 06 | P1 | [06-tool-descriptor-registry.md](06-tool-descriptor-registry.md) | Единый ToolDescriptor registry | in progress (`feat/tool-descriptor-registry`) |
| 07 | P1 | [07-hard-deadline-cancellation.md](07-hard-deadline-cancellation.md) | Hard deadline + CancellationToken | done (`feat/hard-deadline-cancellation`) |
| 08 | P1 | [08-exit-code-on-run-error.md](08-exit-code-on-run-error.md) | Non-zero exit при stop_reason=error | done (`feat/exit-code-on-run-error`) |
| 09 | P1 | [09-sync-architecture-docs.md](09-sync-architecture-docs.md) | Синхронизировать ARCHITECTURE.md | done |
| 10 | P2 | [10-curate-public-api.md](10-curate-public-api.md) | Curated lib.rs / pub(crate) | done |
| 11 | P2 | [11-split-god-modules.md](11-split-god-modules.md) | Разбить god-modules | in progress (`agent/loop` done) |
| 12 | P2 | [12-legacy-access-mode-hardening.md](12-legacy-access-mode-hardening.md) | Legacy mode без access | done (`0.2.0` option B) |
| 13 | P1 | [13-typed-errors-over-string-sniffing.md](13-typed-errors-over-string-sniffing.md) | Типизированные ошибки вместо поиска по тексту | open |
| 14 | P1 | [14-break-remaining-module-cycles.md](14-break-remaining-module-cycles.md) | Оставшиеся циклы модулей | open |
| 15 | P2 | [15-newtype-ids-and-event-kinds.md](15-newtype-ids-and-event-kinds.md) | Newtype для id и видов audit-событий | open |
| 16 | P2 | [16-config-deny-unknown-fields.md](16-config-deny-unknown-fields.md) | `deny_unknown_fields` на DTO конфигурации | open |
| 17 | P2 | [17-decompose-engine-run-inner.md](17-decompose-engine-run-inner.md) | Декомпозиция `AgentEngine::run_inner` | open |
| 18 | P2 | [18-app-composition-root-hygiene.md](18-app-composition-root-hygiene.md) | Гигиена composition root (`app.rs`) | open |
| 19 | P3 | [19-test-gaps-and-env-isolation.md](19-test-gaps-and-env-isolation.md) | Пробелы в тестах и изоляция env | open |

Связанные документы: [обзор 2026-09-29](review-2026-09.md), [ARCHITECTURE.md](../ARCHITECTURE.md), [CONTRACT.md](../../CONTRACT.md), [FURTHER_FIXES.md](../FURTHER_FIXES.md).
