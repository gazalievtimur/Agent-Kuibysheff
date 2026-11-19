# Команды и поля

Имена команд и полей. Пояснение — в главе по ссылке. При расхождении верен [CONTRACT.md](../../CONTRACT.md).

## Команды

`kbshff` без подкоманды открывает мастер настройки в терминале с приглашением. Мастер может записать ключ в `.env` профиля. См. [Первый прогон](01-nachalo.md#первый-прогон).

`kbshff run` выполняет одну задачу и печатает один JSON `RunOutput` на stdout. Обязательны `--project-root`, `--agent` и обычно `--prompt`. `--prompt` можно опустить, если задан `--continue-from`. Полезные флаги: `--files`, `--home`, `--run-id`, `--max-iterations`, `--max-tokens`, `--max-duration-sec`, `--max-cost`, `--save-chat-history`, `--continue-from`, `--progress stderr`. См. [run](07-vstraivanie.md#run).

`kbshff init <agent-id> --project-root <каталог>` создаёт профиль. `--force` перезаписывает существующий. `--interactive` спрашивает модель и пределы. JSON `RunOutput` не печатает. См. [где живёт агент](02-profil.md).

`kbshff check --project-root <каталог> --agent <id>` проверяет профиль, не запуская задачу. Код 0, когда каждая проверка `ok` или сознательный `skip`. Можно пропустить куски: `--skip-provider`, `--skip-mcp`, `--skip-sandbox`.

`kbshff config --project-root <каталог> --agent <id>` показывает и меняет профиль. `--format text` или `json`. Подкоманды: `show`, `import`, `provider`, `limits`, `access`, `mcp`, `billing`, `event-mcp`, `skill`, `prompt`, `rules`, `tools effective`. `import --from` забирает внешний каталог. `--settings-only` обновляет тексты без замены конфига. `--replace-runtime` забирает модель и пределы из посылки. `--force` нужен, чтобы заменить уже заполненный конфиг. См. [профиль](02-profil.md).

`kbshff config … skill body get|set|clear <имя>` читает и пишет `skill.md`. У `set` один источник текста: `--text`, `--file` или `--stdin`.

`kbshff config … skill file list|get|put|remove` работает с сопутствующими файлами умения. У `put` путь внутри папки умения и тот же выбор `--text`, `--file` или `--stdin`.

`kbshff agent list --project-root <каталог>` перечисляет агентов. `--format json` отдаёт машинный список.

`kbshff mcp discover --project-root <каталог> --agent <id>` спрашивает у сохранённых серверов MCP их инструменты.

`kbshff ui` запускает `kbshff-studio`. Флаги `--project-root`, `--bind`, `--open` передаются студии. См. [студия](03-studiya.md).

`kbshff-studio` открывает страницу на `127.0.0.1:8790`. `--no-open` не трогает браузер. `--kbshff` указывает, какой CLI вызывать.

`kbshff acp --agent <id>` держит канал редактора или моста. `--project-root` необязателен, если каталог задаёт сессия. Stdout занят протоколом. `RunOutput` эта команда не печатает. См. [ACP](07-vstraivanie.md#acp).

`kbshff a2a --project-root <каталог> --agent <id>` поднимает HTTP для другого агента. `--bind` по умолчанию `127.0.0.1:8787`. `--public-url` попадает в карточку. `--token-env` включает Bearer. Процесс живёт, пока его не остановят, и `RunOutput` на stdout не пишет. См. [A2A](07-vstraivanie.md#a2a).

Команды управления печатают текст для человека и код процесса. Исключения по потокам — `acp` и `a2a`.

## Поля agent-config.yaml

`provider.base_url`, `provider.model`, `provider.api_key_env` задают OpenAI-совместимый чат и имя переменной с ключом. `timeout_ms`, `max_retries`, `retry_base_delay_ms` относятся к этому HTTP-вызову.

`provider.history.max_tail_messages` и `provider.history.max_chars` обрезают хвост диалога для модели. По умолчанию 30 и 200000. См. [История](06-limity-i-zhurnaly.md#история).

`provider.effort` уходит как `reasoning_effort`. `provider.thinking` уходит как `thinking`, в том числе явное `false`. Незаданные поля в запрос не попадают.

`mcp` — список серверов. У записи есть `name`, `transport` (`stdio` или `http`), для процесса — `command` и `args`, для HTTP — `url`. Необязательны `timeout_ms`, `headers`, `env`, `cwd`, блок `auth` для OAuth. См. [MCP](05-instrumenty.md#mcp).

`limits.max_iterations`, `limits.max_tokens`, `limits.max_duration_sec` останавливают прогон. `limits.max_cost` — объект `amount` и `currency`. См. [пределы](06-limity-i-zhurnaly.md).

`billing.provider_id`, `billing.currency`, `billing.source_order` задают учёт цены. `billing.provider_reported` говорит, где в ответе или заголовке искать сумму и в какой она валюте. `billing.catalog_path` — локальный прайс. `billing.mcp.target` — калькулятор, скрытый от модели. `billing.on_unpriced` по умолчанию `continue`. См. [Стоимость](06-limity-i-zhurnaly.md#стоимость).

`logging.enable_ai_log`, `logging.enable_mcp_log`, `logging.enable_chat_history` включают три журнала. `logging.audit_redaction` чистит JSONL аудита. `logging.sink.type` сейчас `file`. `logging.sink.path` и переменная `AGENT_LOG_DIR` меняют каталог. См. [Журналы](06-limity-i-zhurnaly.md#журналы).

`access.mode` равен `strict`, если поле опущено. `legacy` — широкие файлы без `home.run`, и рядом нельзя задавать остальные гранты. `access.tools.builtins` — встроенные имена. `access.filesystem.home.read` и `write` — префиксы рабочей папки. `access.filesystem.workspace.root` и `read` — корень поиска. `access.filesystem.input_roots` — каталоги для `--files`. `access.run.programs` — псевдонимы `name`, файл `executable`, необязательные `inherit_env`, `runtime_read_roots`, `allow_children`. Рядом потолки `max_processes`, `max_args`, `max_arg_chars`, `max_output_chars`, `max_timeout_ms`. См. [доступ](04-dostup.md).

`event_mcp.events` связывает стадию со списком `handlers`. У обработчика есть `id`, `target`, `timeout_ms`, `on_error` (`abort` или `continue`). Исполняются `context.before_model`, `model.after_response`, `run.before_output`. См. [Event-MCP](07-vstraivanie.md#event-mcp).

`context.pinned_files` — пути относительно рабочей папки. Они перечитываются перед каждым запросом модели и не выпадают при обрезке хвоста.

Комментарии ко всем ключам: [agent-config.example.yaml](../../agent-config.example.yaml).

## stop_reason

`goal_reached` — агент закончил задачу. Код процесса 0.

`limit_reached` — сработал предел шагов, токенов, времени или денег. Код процесса 0.

`error` — прогон сломался. Код процесса ненулевой. Текст причины в `result`. JSON на stdout есть и в этом случае.

## Куда смотреть примеры

Тексты для импорта: [`settings/`](../../settings/). Учебные профили: [test-agents/README.md](../../test-agents/README.md). Готовые формулировки `--prompt`: [prompt-examples.md](../../prompt-examples.md).
