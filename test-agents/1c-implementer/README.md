# 1c-implementer

1c-live stage 4: coder sources → apply-ready `out/cfe/` using agreements identifiers; copy `in/cfe-tests/` → `out/cfe-tests/` without rewriting tests. Orchestrator copies via `adapters/default/apply-out.ps1` (or `adapters/<product>/`) and optionally runs BuildCfe.
