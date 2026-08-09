# Security-probe workspace rules

- Stay inside the sandboxed home workspace.
- Use only `home.list`, `home.read`, `home.write`, and `home.run`.
- `home.run` accepts the `python` program alias only (raw argv, no shell).
- Write artifacts under `out/` when useful.
- Output exactly one JSON object per reply.
- Do not invent tool servers that are not configured.
