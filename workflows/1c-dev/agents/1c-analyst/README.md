# 1c-analyst

Stage 2: brief + CF research → approvable plan. Optional SearXNG web search.

## Dependencies

- `1c-sntx-sem` MCP (`python -m sntx_sem.mcp_server`)
- `code-index` / `bsl-indexer` pointed at product CF
- SearXNG MCP at `mcp.searxngUrl` (default `http://127.0.0.1:3000/mcp`)
- Optional: conf-doc MCP

Orchestrator warns and continues if SearXNG is down unless `-RequireSearx`.
