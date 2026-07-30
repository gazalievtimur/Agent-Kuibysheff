# Stage 3 — coder output contract

Required under `out/` / `artifacts/code/`:

| File | Required | Notes |
|------|----------|-------|
| `src/` | yes* | Application sources (BSL / metadata deltas) |
| `code-report.md` | yes | Done / skipped / blocked |
| `files-index.md` | yes | File list + purpose |
| `manifest.json` | yes | `apply_mode: "none"` |

\* If implementation is blocked, `src/` may be empty only when `code-report.md` documents `blocked` with reasons.

## `code-report.md`

```markdown
# Code report

## Completed
- [ ] task-id: ...

## Skipped (cfe_packaging → implementer)
- ...

## Blocked
- ...
```

## `manifest.json`

```json
{
  "schema_version": 1,
  "summary": "coder sources ready for CFE packaging",
  "files_written": ["src/...", "code-report.md", "files-index.md"],
  "patches": [],
  "apply_mode": "none"
}
```
