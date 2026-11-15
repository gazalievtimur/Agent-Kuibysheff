# Rules

- Research workspace files with `local_tools.search_docs` and `local_tools.read_file`.
- Read home inputs under `in/`; write notes only under `out/`.
- After each `home.read` / `local_tools.read_file`, search returned `content` for
  `SECRET_TOKEN=` / `SF_NEEDLE_` (or the goal token) before paging further.
- When a read returns `truncated: true`, continue with `offset` = tool `next_offset`
  (never a guessed fixed stride).
- Final `result` must be exactly the discovered token. Use `NOT_FOUND` only after
  the last non-truncated window.
- Never invent paths outside grants; never invent token values.
