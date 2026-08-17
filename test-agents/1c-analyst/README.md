# 1c-analyst

1c-live stage 1: brief + CF research → frozen `out/agreements.md` + approvable plan. Optional SearXNG web search.

Identifiers from `in/agreements.json` are literals (see [`../1c-shared/agreements-protocol.md`](../1c-shared/agreements-protocol.md)).

## Dependencies

- `1c-sntx-sem` MCP (`python -m sntx_sem.mcp_server`)
- `code-index` / `bsl-indexer` pointed at product CF
- SearXNG MCP at `mcp.searxngUrl` (default `http://127.0.0.1:3000/mcp`)
- Optional: conf-doc MCP

Orchestrator warns and continues if SearXNG is down unless `-RequireSearx`.
