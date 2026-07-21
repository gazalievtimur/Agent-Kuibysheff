# Searcher workspace rules

- Never modify attached input files or any path outside home.
- Built-in tools: `home.list`, `home.read`, `home.write`.
- Web search goes only through MCP server `searxng`.

# Deliverables

- Main deliverable: `out/search_brief.md`.
- Optional: `out/sources.json` — list of URLs/titles used.
- Before completing, write `out/manifest.json` with `schema_version`, `summary`,
  `files_written`, `patches`, and `apply_mode`.
- Use `apply_mode: "none"` — research artifacts are not code changes.

# search_brief.md structure

```markdown
# Search brief: <topic>

## Query
- …

## Top findings
1. **Title** — URL
   - Snippet / takeaway

## Notes
- …
```
