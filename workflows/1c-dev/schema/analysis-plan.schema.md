# Stage 2 — analysis plan contract

Required under `out/` / `artifacts/plan/`:

| File | Required | Notes |
|------|----------|-------|
| `prd.md` | yes | PRD-lite |
| `architecture.md` | yes | CF vs CFE, objects |
| `tasks.md` | yes | Atomic steps for coder / packaging |
| `cfe-scope.md` | yes | Extension scope for implementer |
| `manifest.json` | yes | `apply_mode: "none"` |
| `phase0-complexity.md` | recommended | simple \| medium \| hard \| critical |
| `requirements.md` | recommended | |
| `codebase-findings.md` | recommended | |
| `adr.md` | if not simple | |
| `workflow-state.md` | recommended | gate state |

## `tasks.md` step labels

Each step must mark executor:

- `bsl` / `metadata` → `1c-coder`
- `cfe_packaging` → `1c-implementer`

## Gate

Human must approve before stage 3: create `artifacts/plan/APPROVED` or pass `-ApprovePlan`.

## `manifest.json`

```json
{
  "schema_version": 1,
  "summary": "analysis plan ready for approval",
  "files_written": ["prd.md", "architecture.md", "tasks.md", "cfe-scope.md"],
  "patches": [],
  "apply_mode": "none"
}
```
