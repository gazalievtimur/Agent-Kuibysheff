# Test agents

Каталог **шаблонов** профилей для `agent_Kuibysheff`. Каждый агент — отдельная
папка с `master_prompt.md` / `skills.dsl` / `rules.md` и примером
`agent-config*.yaml`.

В runtime эти папки **не** передаются как `--settings-dir`. Их импортируют в
protected store:

```powershell
cargo run --bin agent_Kuibysheff -- init referent --project-root . --force
cargo run --bin agent_Kuibysheff -- config --project-root . --agent referent `
  import --from .\test-agents\referent --force
```

CLI остаётся stateless worker: артефакты пишутся в home под `.kuibysheff/homes/<id>/`
(или относительный `--home`), оркестратор решает, как их применять.

## Новый шаблон

```powershell
# Создать защищённый профиль, затем скопировать файлы наружу как шаблон (опционально)
cargo run --bin agent_Kuibysheff -- init my-agent --project-root . --force
# или скопировать существующий каталог test-agents/<id> вручную
```

## Структура шаблона

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

cargo run --bin agent_Kuibysheff -- init referent --project-root . --force
cargo run --bin agent_Kuibysheff -- config --project-root . --agent referent `
  import --from .\test-agents\referent --force

cargo run --bin agent_Kuibysheff -- run `
  --project-root . `
  --agent referent `
  --prompt "Собери первичную информацию по задаче PROJ-123"
```

Для работы с изображениями используйте vision-модель в `provider.model`
(например, `gpt-4o`, `gpt-4.1`, `claude-sonnet-4` и аналоги с поддержкой изображений).

## Advent of Code eval (Referent)

Referent также умеет решать AoC-задачи: MCP `aoc` (`mcp-aoc-tasks.js`) выдаёт
условие и input, код пишется через `home.write`, запускается через `home.run`,
ответ сравнивается по полю `RunOutput.result`.

База заданий и прогоны **не в git** — см. [local/README.md](../local/README.md).

Оркестратор: [`scripts/aoc-eval.ps1`](../scripts/aoc-eval.ps1) (импортирует шаблон в
`local/aoc-eval-project/.kuibysheff/protected/agents/…`).

## Scale-FS probe

`test-agents/scale-fs-probe/` — profile for live regression on large FS corpora
(many-file search, windowed `home.read`). Harness:
`scripts/scale-fs-regression.*` (requires local `workflows/scale-fs-live/`
restored from git history). See [local/README.md](../local/README.md).
