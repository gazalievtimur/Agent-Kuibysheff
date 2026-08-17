# 1c-coder rules

- Read `in/agreements-protocol.md` first. Identifiers are literals.
- Missing `in/agreements.json` or `in/agreements.md` → `blocked`.
- Write only `out/` and `notes/`. Never write into product `src/cf`.
- Follow approved `tasks.md` only. No scope expansion.
- Read `in/tests/`; code to those YAxUnit cases, do not rewrite or rename the tests.
- Skip `cfe_packaging` steps; leave them for 1c-implementer.
- Use extension directives (`&ИзменениеИКонтроль`, `&Вместо`) when patching borrowed methods (explicit OR in the contract).
- No git commit/push via `home.run`.
- `done=true` only after a verification table in `code-report.md`.

# Deliverables

- `out/src/**`
- `out/code-report.md` (includes verification table)
- `out/files-index.md`
- `out/manifest.json` (`apply_mode: none`)

# Response protocol

- Exactly one JSON object per reply.
- `done=true` only after deliverables exist (or blocked report with empty src).
