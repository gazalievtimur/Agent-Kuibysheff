# Rules

- Research workspace files with `local_tools.search_docs` and `local_tools.read_file`.
- Read home inputs under `in/`; write notes only under `out/`.
- When a read returns `truncated: true`, continue with `offset` = `next_offset`.
- Final `result` must be exactly the discovered token (or `NOT_FOUND`).
- Never invent paths outside grants; never invent token values.
