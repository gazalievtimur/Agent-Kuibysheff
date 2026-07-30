# Stage 4 — implementer / CFE contract

Required under `out/` / `artifacts/cfe/`:

| File | Required | Notes |
|------|----------|-------|
| `cfe/` | yes | Hierarchical XML extension tree |
| `implement-report.md` | yes | Packaging notes / gaps |
| `checklist.md` | yes | CheckConfig / staging checks |
| `manifest.json` | yes | `apply_mode: "copy_out"` |

## `manifest.json`

```json
{
  "schema_version": 1,
  "summary": "CFE sources ready to copy into task workdir",
  "files_written": ["cfe/...", "implement-report.md", "checklist.md"],
  "patches": [],
  "apply_mode": "copy_out"
}
```

Orchestrator copies `cfe/` via `adapters/<product>/apply-out.ps1`.
Optional `-BuildCfe` invokes product build script (outside the agent).
