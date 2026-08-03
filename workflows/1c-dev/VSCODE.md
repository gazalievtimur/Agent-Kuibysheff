# 1С-воркфлоу в VS Code: проект + `.kuibyshev`

Открываете в VS Code **папку продукта** (например `C:\ПервыйБИТ\Первый.Гит\ЗУП`), а не репозиторий Kuibyshev. Настройки агентов (MCP, `workspace.root`, prompts) лежат в проекте и правятся под него. Ядро агента универсально и не знает про конкретные MCP.

```text
ЗУП/                          ← VS Code workspace / --project-root
  src/cf/
  .kuibyshev/
    product.yaml
    agents/1c-{intake,analyst,coder,implementer}/
    runs/vscode-active/
  .vscode/settings.json       ← acp.agents
```

CLI и ACP делят один флаг/контекст:

```text
--project-root <ЗУП>
```

Относительные `--config` / `--settings-dir` / `--home` резолвятся как `{project}/.kuibyshev/<path>`.  
В ACP непустой `session/new` → `cwd` имеет приоритет над `--project-root`.

---

## 1. Один раз: установить Kuibyshev

```powershell
cd C:\Git
git clone https://github.com/gybson63/Agent-Kuibyshev.git "Agent Kuibyshev"
cd "Agent Kuibyshev"
cargo build --release --bin agent_Kuibyshev
# добавьте target\release в PATH
```

Ключ LLM — в окружении пользователя / `.env` (ACP сам `.env` не грузит).

Шаблоны профилей остаются в `test-agents/1c-*` (эталон в установке).

---

## 1b. Расширение VS Code (рекомендуется)

В репозитории есть расширение [`extensions/vscode/`](../../extensions/vscode/) — sidebar для параметров агентов, scaffold и prepare/promote/approve/validate.

```powershell
cd "C:\Git\Agent Kuibyshev\extensions\vscode"
npm install
npm run compile
```

Запуск из репозитория Kuibyshev: **Run and Debug → Run Kuibyshev Extension** (F5), либо установите локальный `.vsix`.

В настройках workspace продукта укажите:

- `kuibyshev.repoRoot` — путь к установке Kuibyshev
- `kuibyshev.binaryPath` — при необходимости (по умолчанию `agent_Kuibyshev` из PATH)

Дальше в sidebar **Kuibyshev**: Scaffold → Edit agent (provider/MCP/`workspace.root`) → Validate → Prepare stage → чат с ACP-агентом. Tasks в `.vscode/tasks.json` остаются запасным вариантом.

Подробности: [`extensions/vscode/README.md`](../../extensions/vscode/README.md).

---

## 2. Scaffold в папку продукта

CLI (если без расширения):

```powershell
cd "C:\Git\Agent Kuibyshev"
.\scripts\1c-dev-scaffold-project.ps1 -ProjectRoot "C:\ПервыйБИТ\Первый.Гит\ЗУП"
```

Скрипт создаёт:

| Путь | Назначение |
|------|------------|
| `.kuibyshev/agents/1c-*` | settings-dir + `agent-config.yaml` (копия из test-agents) |
| `.kuibyshev/product.yaml` | пути продукта для prepare/оркестратора |
| `.vscode/settings.json` | четыре ACP-агента с `--project-root ${workspaceFolder}` |
| `.vscode/tasks.json` | prepare / promote / approve |
| запись в `.gitignore` | `.kuibyshev/runs/` |

Дальше **обязательно** отредактируйте в каждом `agents/*/agent-config.yaml`:

- `provider.*` и `api_key_env`
- MCP (`command`, `args`, `url`, env) — как нужно **этому** проекту
- `access.filesystem.workspace.root` — путь к выгрузке CF (scaffold ставит `../../../src/cf` относительно каталога конфига)

Ядро **не** подменяет аргументы MCP.

---

## 3. Работа в VS Code

1. File → Open Folder → папка ЗУП.
2. Через расширение (**Kuibyshev** sidebar) или вручную: Prepare stage / Tasks / CLI.
3. ACP Client → агенты `1c-intake` / `1c-analyst` / `1c-coder` / `1c-implementer`.
4. Prepare (расширение **Prepare stage**, Task **1c: prepare stage**, или CLI):

```powershell
& "C:\Git\Agent Kuibyshev\scripts\1c-dev-acp-prepare.ps1" `
  -ProjectRoot "C:\ПервыйБИТ\Первый.Гит\ЗУП" `
  -RepoRoot "C:\Git\Agent Kuibyshev" `
  -IssueKey PROJ-42 `
  -TaskFile "C:\ПервыйБИТ\Первый.Гит\ЗУП\PROJ-42\tz.md" `
  -Stage 2
```

5. Чат с нужным агентом, стартовая фраза из `CHAT_STARTER.txt` (команда расширения **Copy chat starter**):

```text
Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn.
```

6. Promote / validate → approve plan → stage 3 → 4.

Homes: `.kuibyshev/runs/vscode-active/stageN/home`.  
Apply/BuildCfe по-прежнему через `1c-dev-run.ps1 -ProjectRoot … -ApplyOut` / adapter.

---

## 4. CLI с `--project-root`

```powershell
agent_Kuibyshev run `
  --project-root "C:\ПервыйБИТ\Первый.Гит\ЗУП" `
  --config agents/1c-analyst/agent-config.yaml `
  --settings-dir agents/1c-analyst `
  --home runs/vscode-active/stage2/home `
  --prompt "…"
```

Эквивалент абсолютных путей под `.kuibyshev/…`.

Оркестратор:

```powershell
.\scripts\1c-dev-run.ps1 `
  -Product my-product `
  -ProjectRoot "C:\ПервыйБИТ\Первый.Гит\ЗУП" `
  -IssueKey PROJ-42 `
  -TaskFile "…\tz.md" `
  -Stage 2
```

При наличии `.kuibyshev/agents` используются проектные профили; runs пишутся в `.kuibyshev/runs/`.

---

## 5. Чеклист

- [ ] `agent_Kuibyshev` в PATH
- [ ] Расширение собрано / запущено (или tasks.json как fallback)
- [ ] `kuibyshev.repoRoot` указывает на установку Kuibyshev
- [ ] Scaffold выполнен, `agent-config.yaml` поправлены под MCP/CF проекта
- [ ] VS Code открыт на папке продукта
- [ ] ACP видит четыре агента `1c-*`
- [ ] Prepare stage отрабатывает

Полная установка MCP/инструментов на машине: [SETUP.md](SETUP.md). Контракты артефактов: [schema/](schema/).
