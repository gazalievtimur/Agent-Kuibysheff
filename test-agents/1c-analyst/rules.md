# 1c-analyst rules

- Read `in/agreements-protocol.md` first. Identifiers are literals.
- No `in/agreements.json` → `blocked`. Do not invent names.
- First deliverable: `out/agreements.md` (verbatim backticks from expect/brief). Then the rest of the plan.
- Copy identifiers character-for-character. No synonyms, translation, or “clearer” test names.
- Brief vs expect mismatch → `blocked`.
- Repeat the gate procedure name verbatim in `tasks.md` (in addition to `agreements.md`).
- Write only under `out/` and `notes/`. Never modify product `src/cf` or extensions.
- Prefer CFE for runtime deltas; document CF vs CFE in `architecture.md`.
- Label every `tasks.md` step: `bsl`, `metadata`, or `cfe_packaging`.
- SearXNG: cite URLs in findings/prd; do not treat search hits as requirements.
- Ambiguities → open questions in `requirements.md`, not silent assumptions (mark `assumption:` if unavoidable).
- Human gate is outside this agent; set `workflow-state.md` accordingly.
- `done=true` only after a verification table (agreement → file).

# Required deliverables

- `agreements.md`, `prd.md`, `architecture.md`, `tasks.md`, `cfe-scope.md`, `manifest.json` (`apply_mode: none`)

# Recommended

- `phase0-complexity.md`, `requirements.md`, `codebase-findings.md`, `workflow-state.md`
- `adr.md` when complexity is not simple

# Response protocol

- Exactly one JSON object per reply.
- `done=true` only after required files exist **and** the verification table is written.
