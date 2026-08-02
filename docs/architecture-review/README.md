# Architecture review backlog

Индекс пунктов из архитектурной оценки `agent_Kuibyshev` (2026-08-02).
Каждый пункт — отдельный файл; работаем по одному.

| ID | Sev | Файл | Тема | Status |
|----|-----|------|------|--------|
| 01 | P0 | [01-mcp-stdio-ndjson-framing.md](01-mcp-stdio-ndjson-framing.md) | MCP stdio: NDJSON вместо Content-Length | done |
| 02 | P0 | [02-mcp-stdio-child-shutdown.md](02-mcp-stdio-child-shutdown.md) | Явный shutdown stdio child | done |
| 03 | P0 | [03-audit-log-vs-tool-side-effect.md](03-audit-log-vs-tool-side-effect.md) | Audit-log не маскирует side effect | done |
| 04 | P1 | [04-break-mcp-tools-cycle.md](04-break-mcp-tools-cycle.md) | Разорвать цикл mcp ↔ tools | open |
| 05 | P1 | [05-break-config-access-cycle.md](05-break-config-access-cycle.md) | Разорвать цикл config ↔ access | open |
| 06 | P1 | [06-tool-descriptor-registry.md](06-tool-descriptor-registry.md) | Единый ToolDescriptor registry | open |
| 07 | P1 | [07-hard-deadline-cancellation.md](07-hard-deadline-cancellation.md) | Hard deadline + CancellationToken | open |
| 08 | P1 | [08-exit-code-on-run-error.md](08-exit-code-on-run-error.md) | Non-zero exit при stop_reason=error | open |
| 09 | P1 | [09-sync-architecture-docs.md](09-sync-architecture-docs.md) | Синхронизировать ARCHITECTURE.md | open |
| 10 | P2 | [10-curate-public-api.md](10-curate-public-api.md) | Curated lib.rs / pub(crate) | open |
| 11 | P2 | [11-split-god-modules.md](11-split-god-modules.md) | Разбить god-modules | open |
| 12 | P2 | [12-legacy-access-mode-hardening.md](12-legacy-access-mode-hardening.md) | Legacy mode без access | open |

Связанные документы: [ARCHITECTURE.md](../ARCHITECTURE.md), [CONTRACT.md](../../CONTRACT.md), [FURTHER_FIXES.md](../FURTHER_FIXES.md).
