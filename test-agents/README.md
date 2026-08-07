# Test agents

Каталог описаний тестовых агентов для `agent_Kuibysheff`. Каждый агент — отдельная
папка с `--settings-dir` и примером runtime-конфигурации.

Агенты из этого каталога участвуют в интеграционных и сценарных тестах оркестратора.
CLI остаётся stateless worker: агент пишет артефакты в `--home`, оркестратор решает,
как их применять.

## Новый агент

Чтобы положить профиль в этот каталог:

```powershell
cargo run --bin agent_Kuibysheff -- init my-agent --path .\test-agents\my-agent
# или с запросом provider/limits:
cargo run --bin agent_Kuibysheff -- init my-agent --path .\test-agents\my-agent -i
```

По умолчанию `init` создаёт `./<agent-id>/` в текущей директории.

## Структура агента

```text
test-agents/<agent-id>/
  master_prompt.md          роль и протокол ответа
  skills.dsl                разрешённые инструменты
  rules.md                  правила workspace и deliverables
  agent-config.example.yaml пример provider + MCP + limits
  prompt.example.md         шаблон --prompt для тестов
```

## Запуск (Jira / Confluence research)

```powershell
$env:JIRA_URL = "https://your-company.atlassian.net"
$env:JIRA_USERNAME = "you@company.com"
$env:JIRA_API_TOKEN = "..."
$env:CONFLUENCE_URL = "https://your-company.atlassian.net/wiki"
$env:CONFLUENCE_USERNAME = "you@company.com"
$env:CONFLUENCE_API_TOKEN = "..."

cargo run --bin agent_Kuibysheff -- run `
  --config .\test-agents\referent\agent-config.example.yaml `
  --settings-dir .\test-agents\referent `
  --prompt "Собери первичную информацию по задаче PROJ-123" `
  --home .\demo-home\referent
```

Для работы с изображениями используйте vision-модель в `provider.model`
(например, `gpt-4o`, `gpt-4.1`, `claude-sonnet-4` и аналоги с поддержкой изображений).

## Advent of Code eval (Referent)

Referent также умеет решать AoC-задачи: MCP `aoc` (`mcp-aoc-tasks.js`) выдаёт
условие и input, код пишется через `home.write`, запускается через `home.run`,
ответ сравнивается по полю `RunOutput.result`.

База заданий и прогоны **не в git** — см. [local/README.md](../local/README.md).

```powershell
Copy-Item -Recurse .\local\aoc-bank.example .\local\aoc-bank
.\scripts\aoc-eval.ps1 -TaskId 2024-01-1
```

```bash
cp -R ./local/aoc-bank.example ./local/aoc-bank
./scripts/aoc-eval.sh --task-id 2024-01-1
```

Конфиг: [`referent/agent-config.aoc.example.yaml`](./referent/agent-config.aoc.example.yaml).

## Агенты

| ID | Назначение |
| --- | --- |
| [referent](./referent/) | Research из Jira/Confluence (`mcp-atlassian`) и AoC solve (`mcp-aoc-tasks` + `home.run`) |
| [swebench-solver](./swebench-solver/) | SWE-bench Verified: фикс issue через `workspace.*` MCP в Docker `/testbed` |
| [searxng](./searxng/) | Web search через Streamable HTTP MCP `mcp-searxng` → локальный SearXNG |
| [1c-intake](./1c-intake/) | Этап 1 воркфлоу 1С: Jira/Confluence → brief |
| [1c-analyst](./1c-analyst/) | Этап 2: brief + CF (+ SearXNG) → план |
| [1c-coder](./1c-coder/) | Этап 3: план → `out/src` |
| [1c-implementer](./1c-implementer/) | Этап 4: упаковка CFE `out/cfe` |

Оркестратор 1С: [workflows/1c-dev/README.md](../workflows/1c-dev/README.md), `scripts/1c-dev-run.ps1`.
Оркестратор SWE-bench: [workflows/swebench-verified/README.md](../workflows/swebench-verified/README.md), `scripts/swebench-verified-run.ps1`.

## SearXNG web search

Нужны: SearXNG (JSON API) и `mcp-searxng` на `http://127.0.0.1:3000/mcp`.

```powershell
# SearXNG уже на 127.0.0.1:8080 (пример: контейнер 1c-odata-searxng)
docker run -d --name mcp-searxng -p 3000:3000 `
  --network 1c-odata-skill_default `
  -e MCP_HTTP_PORT=3000 -e MCP_HTTP_HOST=0.0.0.0 `
  -e SEARXNG_URL=http://searxng:8080 `
  isokoliuk/mcp-searxng:latest

New-Item -ItemType Directory -Force -Path .\demo-home\searxng | Out-Null
cargo run --bin agent_Kuibysheff -- run `
  --config .\test-agents\searxng\agent-config.example.yaml `
  --settings-dir .\test-agents\searxng `
  --prompt "Find what SearXNG is and write a short brief to out/search_brief.md" `
  --home .\demo-home\searxng
```


AoC solve regression is part of `scripts/check.ps1` / `scripts/check.sh` via
`scripts/aoc-regression.ps1` / `scripts/aoc-regression.sh`
(see [local/README.md](../local/README.md)).
