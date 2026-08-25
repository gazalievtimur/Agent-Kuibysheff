# Test agents

Каталог **шаблонов** профилей для `agent_Kuibysheff`. Каждый агент — отдельная
папка с `master_prompt.md` / `skills.dsl` / `rules.md` и примером
`agent-config*.yaml`.

В runtime эти папки **не** передаются как `--settings-dir`. Их импортируют в
protected store:

```powershell
cargo run --bin kbshff -- init referent --project-root . --force
cargo run --bin kbshff -- config --project-root . --agent referent `
  import --from .\test-agents\referent --force
```

CLI остаётся stateless worker: артефакты пишутся в home под `.kuibysheff/homes/<id>/`
(или относительный `--home`), оркестратор решает, как их применять.

## Новый шаблон

```powershell
# Создать защищённый профиль, затем скопировать файлы наружу как шаблон (опционально)
cargo run --bin kbshff -- init my-agent --project-root . --force
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

cargo run --bin kbshff -- init referent --project-root . --force
cargo run --bin kbshff -- config --project-root . --agent referent `
  import --from .\test-agents\referent --force

cargo run --bin kbshff -- run `
  --project-root . `
  --agent referent `
  --prompt "Собери первичную информацию по задаче PROJ-123"
```

Для работы с изображениями используйте vision-модель в `provider.model`
(например, `gpt-4o`, `gpt-4.1`, `claude-sonnet-4` и аналоги с поддержкой изображений).

## Advent of Code

AoC offline bank eval and live adventofcode.com orchestration moved to
[kuibysheff-aoc](https://github.com/gybson63/kuibysheff-aoc).
From this repo: `.\scripts\check.ps1 -Aoc` / `./scripts/check.sh --aoc`
(requires a sibling clone or `KUIBYSHEFF_AOC_ROOT`).

Referent here remains the Jira/Confluence research demo.

## Scale-FS probe

`test-agents/scale-fs-probe/` — profile for live regression on large FS corpora
(many-file search, windowed `home.read`). Harness:
`scripts/scale-fs-regression.*` (requires local `workflows/scale-fs-live/`
restored from git history). See [local/README.md](../local/README.md).

## A2A probe

`test-agents/a2a-probe/` — minimal profile for live A2A regression (Agent Card,
Bearer gate, `SendMessage` → write file + result token). Harness:
`scripts/a2a-regression.*` (bank: `local/a2a-bank.example/`). See
[local/README.md](../local/README.md).

## 1C live pipeline (Склад)

Moved to [kuibysheff-1c-live](https://github.com/gybson63/kuibysheff-1c-live)
(analyst → yaxunit → coder → implementer). Product conveyor / VS Code scaffolding
stays here as `1c-intake` + local `workflows/1c-dev/`.
