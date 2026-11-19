# Пределы, стоимость и журналы

Четыре упора останавливают прогон со `stop_reason: limit_reached` и кодом 0. Уже полученный текст остаётся в `result`. Числа задают в `limits` или флагами одного запуска. Флаги перекрывают профиль только на этот вызов.

```powershell
kbshff run --project-root . --agent demo --prompt "Коротко опиши README" --files .\README.md --max-iterations 20 --max-tokens 25000 --max-duration-sec 180 --max-cost USD:0.50
```

| Поле | Значение |
| --- | --- |
| `max_iterations` | Число шагов цикла |
| `max_tokens` | Суммарный бюджет токенов прогона |
| `max_duration_sec` | Время на часах. Ожидание модели и MCP отменяется. Уже начатый инструмент может закончиться |
| `max_cost` | Денежный потолок. В YAML: `{ amount: "1.00", currency: "USD" }`. Во флаге: `USD:0.50` |

`error` и ненулевой код — сбой, не упор в предел.

## История

`provider.history` задаёт хвост диалога для следующего запроса. После поручения и исходного задания остаются последние `max_tail_messages` реплик, весь кусок ограничен `max_chars`. По умолчанию 30 и 200000. Это не `limits.max_tokens`.

`provider.effort` уходит как `reasoning_effort`. `provider.thinking` уходит как `thinking`, в том числе явное `false`. Незаданные поля не отправляются.

`context.pinned_files` — пути внутри рабочей папки. Перед каждым запросом модели они перечитываются и не выпадают при обрезке хвоста.

```yaml
provider:
  history:
    max_tail_messages: 30
    max_chars: 200000
context:
  pinned_files:
    - out/coder_memory.md
```

Если `home.read` или `home.list` в этом прогоне уже получили «файл не найден», тот же путь повторно не спрашивают.

## Стоимость

Каждая физическая попытка запроса, включая повтор, попадает в отчёт. Сумма — десятичная строка. Если цену установить нельзя, статус `partial` или `unavailable`. Ноль не подставляется.

`billing.source_order` задаёт порядок. Обычно это цена провайдера, затем MCP, затем локальный каталог. `billing.mcp.target` указывает на сервер из `mcp`. Модели и Event-MCP этот сервер не показывают. Сбой калькулятора прогон не останавливает. `kbshff check` показывает проблему подключения до задачи.

`--max-cost` не режет прогон жёстко, если попытка осталась без цены. Перерасход может составить один уже завершённый запрос.

`billing.catalog_path` — файл YAML относительно конфига. Правило выбирается по провайдеру, модели, уровню сервиса и окну дат. Совпадение должно быть однозначным.

См. [docs/BILLING.md](../BILLING.md).

## Журналы

По умолчанию каталог `~/.agent-kuibysheff/logs`.

| Файл | Содержание |
| --- | --- |
| `agent.trace.log` | Технический след, дублируется в stderr |
| `ai_usage.jsonl` | Запросы, токены и цена при `logging.enable_ai_log` |
| `mcp_usage.jsonl` | Вызовы MCP при `logging.enable_mcp_log` |

В двух JSONL длинные строки и ключи вроде `api_key`, `token`, `password` вычищаются перед записью. Диалог и `agent.trace.log` эта чистка не затрагивает.

Каталог меняют через `AGENT_LOG_DIR` или `logging.sink.path`. `logging.sink.type` в этом выпуске — `file`.

Полный диалог включают флагом или `logging.enable_chat_history`:

```powershell
kbshff run --project-root . --agent demo --prompt "Продолжи черновик" --save-chat-history
kbshff run --project-root . --agent demo --continue-from .kuibysheff\homes\demo\logs\chat_history.json --save-chat-history
```

`--continue-from` указывает на `chat_history.json` схемы 2. Новый `--prompt` дописывает реплику. Поручение берётся из текущего профиля. Пределы считаются только на продолжение. Путь остаётся внутри `--project-root` и снаружи `protected`.

См. [docs/LOGGING.md](../LOGGING.md).
