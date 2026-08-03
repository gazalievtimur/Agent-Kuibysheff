# Как собрать линию 1С-воркфлоу на другом компьютере

Пошаговая установка конвейера **intake → analyst → coder → implementer** на базе Agent Kuibyshev. Целевая ОС для полного цикла (Designer / CFE) — **Windows**. Linux/WSL подойдёт для этапов 1–3 без `-BuildCfe`, если MCP и пути настроены.

Краткий обзор этапов и CLI: [README.md](README.md).

---

## 0. Что получится в итоге

На машине будут:

| Компонент | Назначение |
|-----------|------------|
| Репозиторий Agent Kuibyshev | CLI-воркер + оркестратор `scripts/1c-dev-run.ps1` (+ VS Code: scaffold `.kuibyshev` в папке продукта — [VSCODE.md](VSCODE.md)) |
| 4 профиля агентов | `test-agents/1c-{intake,analyst,coder,implementer}/` |
| Адаптер продукта | `workflows/1c-dev/products/<id>.yaml` (из `*.yaml.example`) |
| LLM API | OpenAI-compatible `/chat/completions` |
| MCP Atlassian | Jira + Confluence (этап 1) |
| MCP 1c-syntax-sem | справка платформы (этапы 2–4) |
| MCP code-index | поиск по выгрузке CF (этапы 2–4) |
| MCP SearXNG | веб-поиск для analyst (этап 2, можно degrade) |
| Выгрузка CF продукта | например `D:/work/my-1c-product/src/cf` |
| (опционально) скрипт BuildCfe | упаковка `.cfe` на стенд |

Оркестратор **не** коммитит в git продукта и **не** меняет статусы Jira.

---

## 1. Системные требования

### Обязательно

- Windows 10/11 (для полного Designer/CFE) или Windows Server
- PowerShell 5.1+ (`powershell -NoProfile -Command "$PSVersionTable.PSVersion"`)
- Git
- Доступ в интернет к LLM API и (для intake) к Jira/Confluence
- Один из вариантов бинарника агента:
  - Rust toolchain (MSRV **1.86+**) и `cargo build --release`, **или**
  - готовый `agent_Kuibyshev` с [GitHub Releases](https://github.com/gybson63/Agent-Kuibyshev/releases)

### Для этапа 1 (Jira/Confluence)

- [uv](https://docs.astral.sh/uv/) (`uvx` в PATH)
- PAT / токены к Jira и Confluence (read-only достаточно)

### Для этапов 2–4 (анализ и код)

- Python 3.10+ (для `1c-sntx-sem`)
- Клон/установка [1c-sntx-sem](https://github.com/) на машине (локальный путь к `config.yaml`)
- `bsl-indexer` / code-index (`bsl-indexer.exe serve --path <cfRoot>`)
- ripgrep (`rg`) и `git` в PATH (для `home.run` / research)
- Выгрузка конфигурации продукта (hierarchical XML CF)

### Опционально

- Docker Desktop — удобный запуск SearXNG + mcp-searxng
- Платформа 1С + скрипт сборки расширения — только для `-BuildCfe`
- bsl-language-server MCP — lint BSL

---

## 2. Клонирование и сборка агента

```powershell
# Каталог на ваш вкус
cd C:\Git
git clone https://github.com/gybson63/Agent-Kuibyshev.git "Agent Kuibyshev"
cd "Agent Kuibyshev"

# Вариант A: собрать из исходников
cargo build --release --bin agent_Kuibyshev

# Вариант B: положить готовый exe куда угодно и передавать -AgentBin
# .\scripts\1c-dev-run.ps1 ... -AgentBin C:\tools\agent_Kuibyshev.exe
```

Проверка:

```powershell
.\target\release\agent_Kuibyshev.exe --help
# или
cargo run --bin agent_Kuibyshev -- --help
```

Оркестратор сам подхватит `target\release\agent_Kuibyshev.exe`, иначе `target\debug\...`, иначе вызовет `cargo`.

---

## 3. Ключ LLM (.env)

В корне репозитория:

```powershell
Copy-Item .\.env.example .\.env
notepad .\.env
```

Минимум — переменная, на которую ссылается `api_key_env` в конфиге агента.

Примеры:

```env
# Если в agent-config указано api_key_env: OPENAI_API_KEY
OPENAI_API_KEY=sk-...

# Или ваш провайдер (как в .env.example репозитория)
POLZA_API_KEY=...
```

Скрипт `1c-dev-run.ps1` подгружает `.env` через `scripts/import-dotenv.ps1`.  
**Не коммитьте** `.env` и `agent-config.local.yaml`.

В каждом профиле (`test-agents/1c-*/agent-config.*.yaml`) согласуйте:

```yaml
provider:
  base_url: "https://api.openai.com/v1"   # origin OpenAI-compatible API
  model: "gpt-4o"                         # для intake желательно vision
  api_key_env: "OPENAI_API_KEY"
```

---

## 4. Локальные конфиги агентов (обязательно на новой машине)

Файлы `agent-config.example.yaml` содержат **плейсхолдеры путей**. На новом ПК:

```powershell
cd "C:\Git\Agent Kuibyshev"
foreach ($a in @("1c-intake","1c-analyst","1c-coder","1c-implementer")) {
  Copy-Item ".\test-agents\$a\agent-config.example.yaml" `
            ".\test-agents\$a\agent-config.local.yaml"
}
```

Оркестратор **предпочитает** `agent-config.local.yaml`, если файл есть; иначе берёт example.

Дальше правьте **только** `*.local.yaml`:

| Профиль | Что поправить |
|---------|----------------|
| `1c-intake` | `provider.*`; MCP Atlassian (обычно `uvx` без путей) |
| `1c-analyst` | `provider.*`; пути `SNTX_SEM_CONFIG`, code-index `--path`, `workspace.root`, `searxng.url` |
| `1c-coder` | то же для sntx_sem / code-index / workspace |
| `1c-implementer` | то же |

Пример фрагмента analyst local:

```yaml
mcp:
  - name: "sntx_sem"
    command: "python"
    args: ["-m", "sntx_sem.mcp_server"]
    env:
      SNTX_SEM_CONFIG: "D:/tools/1c-sntx-sem/config.yaml"

  - name: "code-index"
    command: "D:/tools/code-index/bsl-indexer.exe"
    args: ["serve", "--path", "D:/work/my-1c-product/src/cf"]

  - name: "searxng"
    transport: http
    url: "http://127.0.0.1:3000/mcp"

access:
  filesystem:
    workspace:
      root: "D:/work/my-1c-product/src/cf"
      read: ["."]
```

---

## 5. Адаптер продукта

```powershell
Copy-Item .\workflows\1c-dev\products\demo.yaml.example `
          .\workflows\1c-dev\products\demo.yaml
notepad .\workflows\1c-dev\products\demo.yaml
```

Заполните пути **этой** машины:

```yaml
workspaceRoot: "D:/work/my-1c-workspace"     # корень workspace с каталогами задач
productRoot: "D:/work/my-1c-workspace/product" # git dump CF
cfSrc: "src/cf"
cfeSrc: "src/cfe"
stagingReleaseBranch: "release/baseline"      # baseline стенда (если есть)
stagingDbPath: "D:/1C/Bases/Staging"          # ИБ для BuildCfe (если нужен)
taskDirPattern: "{workspaceRoot}/{issueKey}"
extensionNamePattern: "Ext_{number}"
mcp:
  codeIndexPath: "{productRoot}/src/cf"
  sntxSemConfig: "D:/tools/1c-sntx-sem/config.yaml"
  searxngUrl: "http://127.0.0.1:3000/mcp"
  codeIndexCommand: "D:/tools/code-index/bsl-indexer.exe"
apply:
  mode: cfe_task_workdir
  buildScript: "{workspaceRoot}/.workflow/skills/cfe-task-extension/scripts/cfe-task-extension.ps1"
```

Локальные `products/*.yaml` с реальными путями **не коммитьте**.

Для ЗУП-подобных раскладок: скопируйте `products/zup.yaml.example` → `products/zup.yaml` и запустите с `-Product zup`.  
Адаптер `adapters/zup/` появится позже; пока apply может потребовать ручной доработки `apply-out`. Общий адаптер — `adapters/default/`.

Проверьте, что каталог CF существует:

```powershell
Test-Path "D:\work\my-1c-product\src\cf"
```

---

## 6. MCP: Atlassian (этап 1)

1. Установите uv: https://docs.astral.sh/uv/
2. Проверьте: `uvx mcp-atlassian --help` (первый запуск скачает пакет)
3. В `.env` или в системе задайте переменные (имена зависят от вашего mcp-atlassian; типичный набор):

```env
JIRA_URL=https://software.example.com
JIRA_PERSONAL_TOKEN=...
CONFLUENCE_URL=https://confluence.example.com
CONFLUENCE_PERSONAL_TOKEN=...
# либо пара username + API token — см. документацию mcp-atlassian
```

4. Альтернатива Docker: образ `ghcr.io/sooperset/mcp-atlassian` + env-файл. Тогда в `1c-intake/agent-config.local.yaml` замените `command`/`args` на вызов `docker run ...` и уберите зависимость от `uvx`, если так удобнее.

5. Smoke без оркестратора (по желанию):

```powershell
$env:JIRA_URL = "..."
# ... остальные env
cargo run --bin agent_Kuibyshev -- run `
  --config .\test-agents\1c-intake\agent-config.local.yaml `
  --settings-dir .\test-agents\1c-intake `
  --prompt "Собери первичную информацию по задаче PROJ-123" `
  --home .\workflows\1c-dev\runs\manual-intake
```

Если Jira на новой машине недоступна — работайте через `-TaskFile` (intake пропускается).

---

## 7. MCP: 1c-syntax-sem (этапы 2–4)

1. Склонируйте и настройте `1c-sntx-sem` (индекс справки платформы).
2. Установите зависимости (`pip install -e .` или по README проекта).
3. Убедитесь, что работает:

```powershell
$env:SNTX_SEM_CONFIG = "D:\tools\1c-sntx-sem\config.yaml"
python -m sntx_sem.mcp_server
# Ctrl+C после успешного старта
```

4. Пропишите тот же `SNTX_SEM_CONFIG` в local-конфигах analyst/coder/implementer.

---

## 8. MCP: code-index (этапы 2–4)

1. Установите `bsl-indexer.exe` (или ваш бинарь code-index) в известный путь.
2. Проверка:

```powershell
& "D:\tools\code-index\bsl-indexer.exe" serve --path "D:\work\my-1c-product\src\cf"
```

3. В local-конфигах укажите тот же `command` и `--path` к CF.
4. В `access.filesystem.workspace.root` — тот же корень CF (для `local_tools`).

Имена tools в `skills.dsl` должны совпадать с тем, что отдаёт `tools/list` вашего indexer (при расхождении поправьте `skills.dsl`).

---

## 9. MCP: SearXNG (этап 2)

Минимальный вариант (Docker):

```powershell
# 1) SearXNG (если ещё нет)
# 2) MCP-обёртка:
docker run -d --name mcp-searxng -p 3000:3000 `
  --add-host=host.docker.internal:host-gateway `
  -e MCP_HTTP_PORT=3000 -e MCP_HTTP_HOST=0.0.0.0 `
  -e SEARXNG_URL=http://host.docker.internal:8080 `
  isokoliuk/mcp-searxng:latest
```

Подробности запуска SearXNG — в [test-agents/searxng](../../test-agents/searxng/) (если каталог есть) и в `products/*.yaml` → `mcp.searxngUrl`.

- Если SearXNG недоступен, этап 2 по умолчанию **продолжается с предупреждением**.
- Для CI/строгого режима: `-RequireSearx`.

---

## 10. (Опционально) BuildCfe / стенд

Нужно только для `-ApplyOut` / `-BuildCfe`:

1. Workspace продукта со скриптом сборки расширения (путь в `buildScript`).
2. Установленная платформа 1С, путь к ИБ в `stagingDbPath`.
3. Права на запись в `{workspaceRoot}/{issueKey}/_work/src`.

Без стенда можно остановиться на артефактах в `workflows/1c-dev/runs/<runId>/artifacts/cfe/` и перенести их вручную.

---

## 11. Чеклист перед первым прогоном

Отметьте на новом ПК:

- [ ] `cargo build --release` или задан `-AgentBin`
- [ ] `.env` с ключом LLM
- [ ] скопированы 4× `agent-config.local.yaml`, пути поправлены
- [ ] `products/demo.yaml` (или свой продукт) с реальными путями
- [ ] CF dump существует и читается
- [ ] (intake) `uvx` + токены Jira/Confluence **или** готов `-TaskFile`
- [ ] (analyst+) python + 1c-sntx-sem
- [ ] (analyst+) code-index на CF
- [ ] (желательно) SearXNG на `searxngUrl`
- [ ] `rg` и `git` в PATH

---

## 12. Первый прогон (рекомендуемый порядок)

Рабочая директория — **корень репозитория** Agent Kuibyshev.

### 12.1. Без Jira — от файла задачи

Создайте файл ТЗ, например `D:\tmp\task.md`, затем:

```powershell
cd "C:\Git\Agent Kuibyshev"

.\scripts\1c-dev-run.ps1 `
  -Product demo `
  -TaskFile D:\tmp\task.md `
  -IssueKey PROJ-99999 `
  -Stage 2
```

Ожидание:

- intake **пропущен**, brief в `workflows/1c-dev/runs/<runId>/artifacts/brief/`
- analyst отработал → `artifacts/plan/`
- процесс завершился с **кодом 2** (gate: нужен approve плана)

Просмотрите `artifacts/plan/{prd,architecture,tasks,cfe-scope}.md`.

### 12.2. Утверждение плана и дальше

```powershell
.\scripts\1c-dev-run.ps1 `
  -Product demo `
  -TaskFile D:\tmp\task.md `
  -IssueKey PROJ-99999 `
  -RunId <тот_же_runId> `
  -FromStage 3 `
  -ApprovePlan
```

Ожидание: `artifacts/code/` и `artifacts/cfe/`, код выхода **0**.

Копирование в каталог задачи продукта:

```powershell
.\scripts\1c-dev-run.ps1 `
  -Product demo `
  -IssueKey PROJ-99999 `
  -RunId <runId> `
  -FromStage 4 `
  -ApprovePlan `
  -ApplyOut
  # и при необходимости -BuildCfe
```

### 12.3. С Jira (полный intake)

```powershell
# env с JIRA_*/CONFLUENCE_* уже в .env или сессии
.\scripts\1c-dev-run.ps1 -Product demo -IssueKey PROJ-123 -Stage 1
.\scripts\1c-dev-run.ps1 -Product demo -IssueKey PROJ-123 -RunId <runId> -FromStage 2
# gate → ApprovePlan → FromStage 3 …
```

### 12.4. Где смотреть результат

```text
workflows/1c-dev/runs/<RunId>/
  artifacts/brief|plan|code|cfe/
  stageN/home/{in,out}/
  logs/stageN.stdout.json
  report.json
```

---

## 13. Типичные проблемы

| Симптом | Что проверить |
|---------|----------------|
| `Product config not found` | Есть ли `workflows/1c-dev/products/<id>.yaml` |
| `Provide -IssueKey or -TaskFile` | Не передан источник задачи |
| `-TaskFile` + `-Stage 1` | Конфликт: intake пропускается |
| Exit code 2 | Это не ошибка — нужен `-ApprovePlan` |
| `stop_reason` ≠ `goal_reached` | `logs/stageN.stdout.json`, лимиты, MCP, ключ API |
| SearXNG warning | Поднимите MCP или работайте без `-RequireSearx` |
| PolicyDenied / нет tools | `skills.dsl` ∩ `access` ∩ MCP `tools/list` |
| Пути CF не читаются | `workspace.root` и `--path` code-index на **этом** диске |
| `SwitchParameter` / странные ошибки PS | Не создавайте переменные `$Debug`, `$ApplyOut` вручную; используйте свежий скрипт из репо |
| BuildCfe падает | Платформа, ИБ закрыта в Конфигураторе, путь `buildScript` |

---

## 14. Минимальный набор «только попробовать линию»

Если нет Jira, стенда и SearXNG:

1. Собрать агент + `.env` с LLM.
2. Положить выгрузку CF и настроить code-index + sntx_sem (иначе analyst/coder сильно деградируют).
3. Скопировать `agent-config.local.yaml`, поправить пути.
4. Скопировать `products/demo.yaml.example` → `demo.yaml` и поправить пути.
5. Запуск: `-TaskFile` → stage 2 → gate → `-ApprovePlan -FromStage 3`.

Без code-index/sntx_sem линия формально запустится, но качество плана и кода будет низким — для «настоящей» машины эти MCP считайте обязательными для этапов 2–4.

---

## 15. Что не переносится «как есть» с вашего текущего ПК

- Абсолютные пути в `products/*.yaml` и `agent-config*.yaml`
- Секреты (`.env`, PAT)
- Локальные индексы 1c-sntx-sem / code-index (нужна своя установка)
- Docker-образы и тома SearXNG
- Путь к ИБ стенда и версии платформы 1С
- Каталог `workflows/1c-dev/runs/` (прогоны, в git не входят)

Переносится из git: оркестратор, профили агентов (prompts/skills/rules), схемы артефактов, example-конфиги и шаблоны продуктов.
