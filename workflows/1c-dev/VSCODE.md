# 1С-воркфлоу в VS Code: проект + `.kuibysheff`

Открываете в VS Code **папку продукта** (например `C:\ПервыйБИТ\Первый.Гит\ЗУП`), а не репозиторий Kuibysheff. Настройки агентов (MCP, `workspace.root`, prompts) лежат в проекте и правятся под него. Ядро агента универсально и не знает про конкретные MCP.

```text
ЗУП/                          ← VS Code workspace / --project-root
  src/cf/
  .kuibysheff/
    product.yaml
    protected/agents/1c-{intake,analyst,coder,implementer}/
    homes/
    mcp-runtime/
    runs/vscode-active/
  .vscode/settings.json       ← acp.agents
```

CLI и ACP адресуют агента так:

```text
--project-root <ЗУП> --agent 1c-analyst
```

Профиль всегда в `.kuibysheff/protected/agents/<id>/` (агент владеет хранилищем).  
Внешние шаблоны заносятся через `config import --from …`.  
В ACP непустой `session/new` → `cwd` имеет приоритет над `--project-root`.

---

## 1. Один раз: установить Kuibysheff

```powershell
cd C:\Git
git clone https://github.com/gybson63/Agent-Kuibysheff.git "Agent Kuibysheff"
cd "Agent Kuibysheff"
cargo build --release --bin agent_Kuibysheff
# добавьте target\release в PATH
```

Ключ LLM — в окружении пользователя / `.env` (ACP сам `.env` не грузит).

Шаблоны профилей остаются в `test-agents/1c-*` (эталон в установке).

---

## 1b. Расширение VS Code (рекомендуется)

В репозитории есть расширение [`extensions/vscode/`](../../extensions/vscode/) — sidebar для параметров агентов, scaffold и prepare/promote/approve/validate.

```powershell
cd "C:\Git\Agent Kuibysheff\extensions\vscode"
npm install
npm run compile
```

Запуск из репозитория Kuibysheff: **Run and Debug → Run Kuibysheff Extension** (F5), либо установите локальный `.vsix`.

В настройках workspace продукта укажите:

- `kuibysheff.repoRoot` — путь к установке Kuibysheff
- `kuibysheff.binaryPath` — при необходимости (по умолчанию `agent_Kuibysheff` из PATH)

Дальше в sidebar **Kuibysheff**: Scaffold → Edit agent (provider/MCP/`workspace.root`) → Validate → Prepare stage → чат с ACP-агентом. Tasks в `.vscode/tasks.json` остаются запасным вариантом.

Подробности: [`extensions/vscode/README.md`](../../extensions/vscode/README.md).

---

## 2. Scaffold в папку продукта

CLI (если без расширения):

```powershell
cd "C:\Git\Agent Kuibysheff"
.\scripts\1c-dev-scaffold-project.ps1 -ProjectRoot "C:\ПервыйБИТ\Первый.Гит\ЗУП"
```

Скрипт создаёт:

| Путь | Назначение |
|------|------------|
| `.kuibysheff/protected/agents/1c-*` | protected-профили (`init` + `config import` из test-agents) |
| `.kuibysheff/product.yaml` | пути продукта для prepare/оркестратора |
| `.vscode/settings.json` | четыре ACP-агента с `--project-root ${workspaceFolder}` |
| `.vscode/tasks.json` | prepare / promote / approve |
| запись в `.gitignore` | `.kuibysheff/runs/` |

Дальше **обязательно** отредактируйте в каждом `protected/agents/*/agent-config.yaml`
(или через `agent_Kuibysheff config …`):

- `provider.*` и `api_key_env`
- MCP (`command`, `args`, `url`, env) — как нужно **этому** проекту
- `access.filesystem.workspace.root` — путь к выгрузке CF (scaffold ставит `../../../src/cf` относительно каталога конфига)

Ядро **не** подменяет аргументы MCP.

---

## 3. Работа в VS Code

1. File → Open Folder → папка ЗУП.
2. Через расширение (**Kuibysheff** sidebar) или вручную: Prepare stage / Tasks / CLI.
3. ACP Client → агенты `1c-intake` / `1c-analyst` / `1c-coder` / `1c-implementer`.
4. Prepare (расширение **Prepare stage**, Task **1c: prepare stage**, или CLI):

```powershell
& "C:\Git\Agent Kuibysheff\scripts\1c-dev-acp-prepare.ps1" `
  -ProjectRoot "C:\ПервыйБИТ\Первый.Гит\ЗУП" `
  -RepoRoot "C:\Git\Agent Kuibysheff" `
  -IssueKey PROJ-42 `
  -TaskFile "C:\ПервыйБИТ\Первый.Гит\ЗУП\PROJ-42\tz.md" `
  -Stage 2
```

5. Чат с нужным агентом, стартовая фраза из `CHAT_STARTER.txt` (команда расширения **Copy chat starter**):

```text
Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn.
```

6. Promote / validate → approve plan → stage 3 → 4.

Homes: `.kuibysheff/runs/vscode-active/stageN/home`.  
Apply/BuildCfe по-прежнему через `1c-dev-run.ps1 -ProjectRoot … -ApplyOut` / adapter.

---

## 4. CLI с `--project-root` + `--agent`

```powershell
agent_Kuibysheff run `
  --project-root "C:\ПервыйБИТ\Первый.Гит\ЗУП" `
  --agent 1c-analyst `
  --prompt "…"
```

Профиль: `.kuibysheff/protected/agents/1c-analyst/`. Home по умолчанию: `.kuibysheff/homes/1c-analyst/`.

Оркестратор:

```powershell
.\scripts\1c-dev-run.ps1 `
  -Product my-product `
  -ProjectRoot "C:\ПервыйБИТ\Первый.Гит\ЗУП" `
  -IssueKey PROJ-42 `
  -TaskFile "…\tz.md" `
  -Stage 2
```

При наличии `.kuibysheff/protected/agents` используются проектные профили; runs пишутся в `.kuibysheff/runs/` / `.kuibysheff/homes/`.

---

## 5. Чеклист

- [ ] `agent_Kuibysheff` в PATH
- [ ] Расширение собрано / запущено (или tasks.json как fallback)
- [ ] `kuibysheff.repoRoot` указывает на установку Kuibysheff
- [ ] Scaffold выполнен, `agent-config.yaml` поправлены под MCP/CF проекта
- [ ] VS Code открыт на папке продукта
- [ ] ACP видит четыре агента `1c-*`
- [ ] Prepare stage отрабатывает

Полная установка MCP/инструментов на машине: [SETUP.md](SETUP.md). Контракты артефактов: [schema/](schema/).
