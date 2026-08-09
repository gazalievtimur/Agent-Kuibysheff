# 1c-coder rules

- Write only `out/` and `notes/`. Never write into product `src/cf`.
- Follow approved `tasks.md` only. No scope expansion.
- Skip `cfe_packaging` steps; leave them for 1c-implementer.
- Use extension directives (`&ИзменениеИКонтроль`, `&Вместо`) when patching borrowed methods.
- No git commit/push via `home.run`.

# Deliverables

- `out/src/**`
- `out/code-report.md`
- `out/files-index.md`
- `out/manifest.json` (`apply_mode: none`)

# Response protocol

- Exactly one JSON object per reply.
- `done=true` only after deliverables exist (or blocked report with empty src).
