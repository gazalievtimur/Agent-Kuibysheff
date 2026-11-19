# Установка

Скачайте архив с [GitHub Releases](https://github.com/gazalievtimur/Agent-Kuibysheff/releases) и положите `kbshff` в `PATH`.

Готовые архивы: Windows x86_64 и Linux x86_64. Имена: `agent_Kuibysheff-vX.Y.Z-x86_64-pc-windows-msvc.zip` и `agent_Kuibysheff-vX.Y.Z-x86_64-unknown-linux-gnu.zip`. Подставьте версию со страницы релиза. Рядом лежит `.zip.sha256`.

Windows:

```powershell
Get-FileHash .\agent_Kuibysheff-vX.Y.Z-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Get-Content .\agent_Kuibysheff-vX.Y.Z-x86_64-pc-windows-msvc.zip.sha256
```

Linux:

```bash
sha256sum -c agent_Kuibysheff-vX.Y.Z-x86_64-unknown-linux-gnu.zip.sha256
```

```text
kbshff --help
```

Сборки Linux сделаны на GitHub `ubuntu-latest` и рассчитаны на glibc этой среды. На более старом дистрибутиве соберите программу из исходников.

macOS и Linux aarch64 в релизах нет.

## Исходники

Нужен Rust 1.88 или новее.

```powershell
git clone https://github.com/gazalievtimur/Agent-Kuibysheff.git
cd Agent-Kuibysheff
cargo build --release --bin kbshff --bin kbshff-studio
```

Файлы появятся в `target\release`. Пока `kbshff` не в `PATH`, вызывайте его полным путём или через `cargo run --bin kbshff --`.

`kbshff-studio` нужен только для браузера. См. [Студия](03-studiya.md).

## Обновление

Новый бинарник заменяет программу. Каталог `.kuibysheff` в проекте не меняется: там профили, рабочие папки и журналы прогонов.

Удаление программы — убрать `kbshff` из `PATH`. Каталог `.kuibysheff` удаляют отдельно. Расширение VS Code снимается в редакторе. Его версия не совпадает с версией `kbshff`.

См. [docs/INSTALL.md](../INSTALL.md).

## Первый прогон

`--project-root` — каталог продукта, не клон Kuibysheff. В нём появится `.kuibysheff`.

Ключ провайдера записывает мастер. Он же создаёт или проверяет профиль:

```powershell
kbshff
```

Без приглашения терминала задайте переменную сами. По умолчанию это `OPENAI_API_KEY`. Значение в текущем окне:

```powershell
$env:OPENAI_API_KEY = "ваш_ключ"
```

Чтобы ключ подхватывался при каждом запуске, после `init` положите его в `.kuibysheff\protected\agents\demo\.env`:

```text
OPENAI_API_KEY=ваш_ключ
```

Ключ не подставляется в текст задания. Имена переменных — в [`.env.example`](../../.env.example).

Создайте агента и выполните задачу:

```powershell
kbshff init demo --project-root .
kbshff check --project-root . --agent demo
kbshff run --project-root . --agent demo --prompt "Кратко перескажи приложенный файл в out/summary.md" --files .\README.md
```

`--prompt` — текст задачи. `--files` — текстовые файлы UTF-8. Их содержимое попадает в запрос к модели и в рабочую папку не копируется. Каждый файл в контексте обрезается до 50 000 символов.

`init` создаёт профиль `demo`. `check` проверяет модель, MCP и песочницу и не запускает задачу. Код 0 означает, что каждая проверка `ok` или сознательно пропущена.

`run` печатает один JSON на stdout. Текст ответа — поле `result`. Успех — `stop_reason` со значением `goal_reached`.

Шаблоны формулировок: [prompt-examples.md](../../prompt-examples.md). Тот же пересказ выполняет `run-demo.ps1`.

Шаблон `init` разрешает чтение и запись в рабочей папке и правку сохранённого профиля через умение `configure`. Поиск по репозиторию и запуск программ выключены. См. [Разрешения](04-dostup.md).

Пустой ключ виден в `result` и в выводе `check`. Сообщение, что блок `access` обязателен, относится к профилю старше 0.2: с этой версии разрешения задают явно. Текущий `init` такой блок уже записывает.

## Результат

`run` печатает один JSON. Текст — в `result`. Файлы, которые записал агент, — в `.kuibysheff/homes/demo`. При старте там создаются `in` и `out`: читать можно оба, писать — в `out`. Пересказ из примера попадает в `out`.

Пути журналов — объект `logs`. По умолчанию каталог `~/.agent-kuibysheff/logs`.

| `stop_reason` | Код | Значение |
| --- | --- | --- |
| `goal_reached` | 0 | Задача завершена |
| `limit_reached` | 0 | Сработал предел шагов, токенов, времени или денег. `result` может быть неполным |
| `error` | ненулевой | Сбой. JSON всё равно на stdout, причина в `result` |

Разбор полей для скрипта: [run, ACP и A2A](07-vstraivanie.md).
