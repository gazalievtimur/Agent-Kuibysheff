# Профиль

Команда `init` создаёт профиль в каталоге проекта. Прогон находит его по `--project-root` и `--agent`. Отдельного флага пути к конфигу нет.

```text
.kuibysheff/
  protected/agents/demo/
    agent-config.yaml
    master_prompt.md
    rules.md
    skills.dsl
    .env
  runtime/skills/demo/
    workspace/
      skill.md
  homes/demo/
    in/
    out/
```

`protected/agents/demo` читает и пишет только Kuibysheff. Входные файлы и файловые инструменты туда не ходят. Инструкции умений лежат в `runtime/skills`, чтобы изолированный запуск мог прочитать скрипт и не видел ключ.

Рабочая папка по умолчанию — `.kuibysheff/homes/demo`. Свой путь задают `--home`: он относительный, остаётся внутри `.kuibysheff` и не указывает в `protected`.

Ключ в `.env` нужен для вызова модели. В `master_prompt.md` и `rules.md` он не подставляется. Ключ в `master_prompt.md` попадает в запрос к модели.

```powershell
kbshff config --project-root . --agent demo show
kbshff agent list --project-root .
```

`show` печатает модель, адрес, число серверов MCP, число умений и пределы. Ключ в сводку не входит.

## Импорт

Внешний каталог записывают в профиль и только потом проверяют:

```powershell
kbshff config --project-root . --agent demo import --from .\settings
```

`--force` заменяет уже заполненный профиль. `--settings-only` обновляет тексты и умения и не трогает `agent-config.yaml`. `--replace-runtime` при импорте конфига забирает `provider` и `limits` из посылки. Без этого флага живые `provider` и `limits` сохраняются.

Готовые тексты: [`settings/`](../../settings/). Учебные профили: [test-agents/README.md](../../test-agents/README.md).

## Тексты

`master_prompt.md` — постоянное поручение на каждый прогон. Задачу одного запуска передают `--prompt`.

`rules.md` — текст ограничений для модели. Какие папки открыты, решает `access`. См. [Разрешения](04-dostup.md).

`skills.dsl` — умения. У каждого одна строка `policy` и список `allowed_tools`. В `policy` нельзя поставить `"`. Длинную инструкцию пишут в `skill.md`.

Шаблон `init` создаёт умение `workspace` (чтение и запись рабочей папки) и `configure` (правка сохранённого профиля). Правка профиля применяется при следующем запуске.

```powershell
kbshff config --project-root . --agent demo prompt set --file .\master_prompt.md
kbshff config --project-root . --agent demo rules set --file .\rules.md
```

## Умения

Инструкция и сопутствующие файлы:

```text
.kuibysheff/runtime/skills/demo/workspace/skill.md
.kuibysheff/runtime/skills/demo/workspace/scripts/example.py
```

Имя папки совпадает с именем в `skills.dsl` и состоит из одного сегмента: буквы, цифры, пробел, `_` или `-`. В запрос к модели попадает оглавление: есть ли инструкция и сколько файлов. Текст `skill.md` модель читает отдельным инструментом.

Скрипт из папки не запускается на месте. Модель читает текст, пишет копию в рабочую папку и выполняет её через `home.run`, если этот инструмент разрешён.

```powershell
kbshff config --project-root . --agent demo skill body set workspace --text "# Шаги"
kbshff config --project-root . --agent demo skill file put workspace scripts/example.py --file .\example.py
kbshff config --project-root . --agent demo skill list
```

`skill body get` показывает `skill.md`, `skill body clear` удаляет его. Сопутствующие файлы: `skill file list`, `get`, `remove`.

Если `runtime/skills/demo` нет, загрузка берёт `protected/agents/demo/skills`. Если есть оба дерева, используется `runtime`. Симлинк или не-каталог на месте `runtime/skills/demo` ломает загрузку.

`import` копирует `skills/<имя>/`, если внутри есть `skill.md` и имя объявлено в `skills.dsl` посылки.

Встроенный инструмент доступен, когда его имя есть и в `access.tools.builtins`, и в `allowed_tools` какого-либо умения. Инструмент MCP доступен, когда умение перечислило полное имя `сервер.инструмент`. На каждом прогоне без записи в `skills.dsl` доступны `agent.list_tools`, `agent.describe_tool` и `agent.guide`. `profile.list_skill` и `profile.read_skill` добавляются, когда папка умения существует.

Формула пересечения: раздел Access policy в [CONTRACT.md](../../CONTRACT.md).
