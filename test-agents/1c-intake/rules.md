# 1c-intake rules

- Never modify paths outside home. Write only under `out/` (and optional `notes/`).
- Built-ins: `home.list`, `home.read`, `home.write` only (no `home.run`).
- Atlassian MCP is read-only. Never claim Jira/Confluence were modified.
- Without Confluence TZ (or explicit TZ-only-in-Jira), set `tz_status: missing` or `partial` and list open questions. Do not claim ready for implementation.
- Order: issue → linked issues → attachments → Confluence links → search fallback → brief.

# Deliverables

- `out/task_brief.md` — see workflows/1c-dev/schema/task-brief.schema.md
- `out/sources.json`
- `out/manifest.json` with `apply_mode: "none"`

# task_brief.md outline

```markdown
# Task brief: <ISSUE-KEY or title>

## Source
- Jira: ...
- Confluence: ...
- Origin: jira_intake

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

# Response protocol

- Exactly one JSON object per reply.
- `done=true` only after required files are written.
