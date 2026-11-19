# Введение

Kuibysheff — CLI-агент. Он принимает задачу, работает в отдельной папке проекта и печатает один JSON.

Программа называется `kbshff`. Страница в браузере — `kbshff-studio`.

## CLI

`kbshff run` выполняет одну задачу и завершается. `kbshff acp` держит канал для редактора или моста. `kbshff a2a` принимает задачи по HTTP.

Готовые архивы есть для Windows x86_64 и Linux x86_64. macOS и Linux aarch64 собирают из исходников.

См. [Установка и первый прогон](01-nachalo.md).

## Профиль

Настройки, тексты и ключ лежат в `.kuibysheff/protected/agents/<id>/`. Записанные файлы — в `.kuibysheff/homes/<id>/`.

См. [Профиль](02-profil.md).

## Документы

- [Установка и первый прогон](01-nachalo.md)
- [Профиль](02-profil.md)
- [Студия](03-studiya.md)
- [Разрешения](04-dostup.md)
- [Инструменты и MCP](05-instrumenty.md)
- [Пределы, стоимость и журналы](06-limity-i-zhurnaly.md)
- [`run`, ACP и A2A](07-vstraivanie.md)
- [Команды и поля](08-spravochnik.md)

Имена полей и коды выхода — в [CONTRACT.md](../../CONTRACT.md). При расхождении верен контракт. Модули программы — в [docs/ARCHITECTURE.md](../ARCHITECTURE.md).
