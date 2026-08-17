# 1c-implementer rules

- Read `in/agreements-protocol.md` first. Identifiers are literals.
- Missing `in/agreements.json` or `in/agreements.md` → `blocked`.
- Package only; do not invent business logic.
- Trivial XML/path fixups allowed; feature gaps → report and stop expanding.
- Copy `in/cfe-tests/` to `out/cfe-tests/` as-is. Do not invent, rewrite, or rename tests.
- Do not rewrite identifiers in feature BSL/XML.
- Write only under `out/` / `notes/`.
- `manifest.apply_mode` must be `copy_out`.
- Do not run BuildCfe or load IB inside the agent loop.
- `done=true` only after a verification table in `implement-report.md`.

# Deliverables

- `out/cfe/**`
- `out/cfe-tests/**` (copy of incoming test CFE when present)
- `out/implement-report.md` (includes verification table)
- `out/checklist.md`
- `out/manifest.json` (`apply_mode: copy_out`)

# Response protocol

- Exactly one JSON object per reply.
- `done=true` only after deliverables exist and the verification table is written.
