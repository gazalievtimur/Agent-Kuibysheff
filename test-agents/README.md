# Test agents

Каталог описаний тестовых агентов для `agent_Kuibyshev`. Каждый агент — отдельная
папка с `--settings-dir` и примером runtime-конфигурации.

Агенты из этого каталога участвуют в интеграционных и сценарных тестах оркестратора.
CLI остаётся stateless worker: агент пишет артефакты в `--home`, оркестратор решает,
как их применять.

## Структура агента

```text
test-agents/<agent-id>/
  master_prompt.md          роль и протокол ответа
  skills.dsl                разрешённые инструменты
  rules.md                  правила workspace и deliverables
  agent-config.example.yaml пример provider + MCP + limits
  prompt.example.md         шаблон --prompt для тестов
```

## Запуск

```powershell
$env:JIRA_URL = "https://your-company.atlassian.net"
$env:JIRA_USERNAME = "you@company.com"
$env:JIRA_API_TOKEN = "..."
$env:CONFLUENCE_URL = "https://your-company.atlassian.net/wiki"
$env:CONFLUENCE_USERNAME = "you@company.com"
$env:CONFLUENCE_API_TOKEN = "..."

cargo run -- `
  --config .\test-agents\referent\agent-config.example.yaml `
  --settings-dir .\test-agents\referent `
  --prompt "Собери первичную информацию по задаче PROJ-123" `
  --home .\demo-home\referent
```

Для работы с изображениями используйте vision-модель в `provider.model`
(например, `gpt-4o`, `gpt-4.1`, `claude-sonnet-4` и аналоги с поддержкой изображений).

## Агенты

| ID | Назначение |
| --- | --- |
| [referent](./referent/) | Сбор первичной информации по задаче из Jira и Confluence через MCP `mcp-atlassian` |
