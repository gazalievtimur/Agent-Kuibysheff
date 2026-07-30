# 1c-implementer rules

- Package only; do not invent business logic.
- Trivial XML/path fixups allowed; feature gaps → report and stop expanding.
- Write only under `out/` / `notes/`.
- `manifest.apply_mode` must be `copy_out`.
- Do not run BuildCfe or load IB inside the agent loop.

# Deliverables

- `out/cfe/**`
- `out/implement-report.md`
- `out/checklist.md`
- `out/manifest.json` (`apply_mode: copy_out`)

# Response protocol

- Exactly one JSON object per reply.
