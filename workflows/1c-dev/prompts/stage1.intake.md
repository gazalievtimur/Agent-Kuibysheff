Собери первичную информацию по задаче {{ISSUE_KEY}} (product={{PRODUCT}}).

Goal: проверяемый brief из Jira/Confluence без анализа кода конфигурации.

Required steps:
1. Fetch the Jira issue and linked Confluence pages through atlassian MCP tools (read-only).
2. Download attachments / images when relevant; describe screenshots if the model supports vision.
3. Write out/task_brief.md with Source, Summary, Requirements and acceptance, Related documentation, Images and attachments, Open questions, tz_status (missing|partial|ok), Raw references.
4. Write out/sources.json with origin, issue_key, jira/confluence citations.
5. Write out/manifest.json with apply_mode=none.
6. Final response: done=true with a short result.

Рамки: не анализируй src/cf, не составляй план реализации, не пиши в Jira/Confluence.

Return JSON only on every turn.
