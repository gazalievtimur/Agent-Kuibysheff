# Referent workspace rules

- Never modify attached input files or any path outside home.
- Use `in/` only when the orchestrator prepared extra context (for example, a cached export).
- Built-in tools: `home.list`, `home.read`, `home.write`, `home.run`.
- `home.run` executes argv with cwd = home (no shell). Prefer `python` for AoC solutions.

# Research deliverables (Jira / Confluence)

- Main deliverable: `out/task_brief.md`.
- Optional supporting files:
  - `out/images/<slug>.md` — one file per described image when vision is available.
  - `out/sources.json` — machine-readable list of fetched Jira/Confluence sources.
- Before completing, write `out/manifest.json` with `schema_version`, `summary`,
  `files_written`, `patches`, and `apply_mode`.
- Use `apply_mode: "none"` — research artifacts are not code changes.
- Never claim that Jira or Confluence were modified unless a write MCP tool was explicitly
  called and the tool result confirms success.

# task_brief.md structure

Use this outline (adapt sections when data is missing):

```markdown
# Task brief: <ISSUE-KEY or title>

## Source
- Jira: <url or key>
- Confluence: <links>

## Summary
<2-5 sentences>

## Requirements and acceptance
- ...

## Related documentation
- ...

## Images and attachments
| Name | Source | Description / note |
| --- | --- | --- |

## Open questions
- ...

## Raw references
- ...
```

# MCP usage (mcp-atlassian)

Discover tools with the runtime tool list. Typical read sequence:

1. `jira_get_issue` — load issue fields, description, links.
2. `jira_search` — find related issues when the prompt asks for context.
3. `confluence_search` / `confluence_get_page` — load linked documentation.
4. `jira_get_issue_images`, `jira_download_attachments`,
   `confluence_get_page_images`, `confluence_download_attachment` — images and files.

When search is needed, keep result sets small (for example, `limit: 10`).

# MCP usage (aoc / Advent of Code)

Typical solve sequence:

1. `aoc_get_task` — load statement (`text`) by `task_id` or `url`.
2. `aoc_get_input` — load puzzle input.
3. `home.write` — save `solution.py` and optionally `input.txt`.
4. `home.run` — `{"program":"python","args":["solution.py"]}`.
5. Iterate on code using stdout/stderr until the answer is correct.
6. Final `result` must be only the answer string (trimmed), matching AoC output style.

The AoC MCP never exposes expected answers. Do not invent puzzle statements or inputs.

# Vision-capable models

When the provider model supports image understanding:

- Describe UI screenshots, diagrams, and photos relevant to the task.
- Note visible text, layout, highlighted areas, and implied user actions.
- Save long descriptions under `out/images/` and link them from `task_brief.md`.

When vision is not available:

- List attachment names, MIME types, and MCP-returned paths or URLs.
- Add a note: `Vision review required`.

# Response protocol

- Output exactly one JSON object per reply — never multiple objects in one message.
- Wait for tool results before the next turn; do not pre-emit future steps as extra JSON.
- Use `done=false` while work is still in progress.
- One main file per `home.write` call when possible.
