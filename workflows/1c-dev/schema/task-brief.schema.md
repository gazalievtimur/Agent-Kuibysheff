# Stage 1 — task brief contract

Required under `out/` (or orchestrator `artifacts/brief/` after normalize):

| File | Required | Notes |
|------|----------|-------|
| `task_brief.md` | yes | Human-readable brief |
| `sources.json` | yes | Origin and citations |
| `manifest.json` | yes | `apply_mode: "none"` |

## `task_brief.md` sections

```markdown
# Task brief: <ISSUE-KEY or title>

## Source
- Jira: ...
- Confluence: ...
- Origin: jira_intake | task_file

## Summary
...

## Requirements and acceptance
- ...

## Related documentation
- ...

## Images and attachments
| Name | Source | Description / note |
| --- | --- | --- |

## Open questions
- ...

## tz_status
missing | partial | ok

## Raw references
- ...
```

## `sources.json`

```json
{
  "origin": "jira_intake",
  "issue_key": "K7-123",
  "skipped_intake": false,
  "jira": [],
  "confluence": [],
  "attachments": []
}
```

When intake is skipped via `-TaskFile`:

```json
{
  "origin": "task_file",
  "path": "C:/path/to/task.md",
  "skipped_intake": true
}
```

## `manifest.json`

```json
{
  "schema_version": 1,
  "summary": "short",
  "files_written": ["task_brief.md", "sources.json"],
  "patches": [],
  "apply_mode": "none"
}
```
