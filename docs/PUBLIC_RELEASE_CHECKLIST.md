# Чеклист подготовки к публичному релизу

Этот документ — барьер допуска перед переводом репозитория в публичный режим.
Он не заменяет исправления: пункт считается закрытым только после проверки
указанного результата.

## Как пользоваться

- `P0` — блокер: публикация запрещена, пока пункт не закрыт.
- `P1` — должно быть закрыто до публикации либо явно принято владельцем как
  исключение с причиной и сроком.
- `P2` — улучшение релиза; допустимо выполнить после открытия.
- `[x]` означает, что механизм уже найден в репозитории. Перед публикацией его
  всё равно нужно повторно проверить на чистом коммите.
- Результаты проверок и принятые исключения следует приложить к release issue
  или другому доступному сопровождающим журналу решений.

## Текущее основание

На момент первичного аудита в репозитории уже есть:

- [x] лицензия Apache-2.0 и файл [`NOTICE`](../NOTICE);
- [x] DCO-требование `Signed-off-by` в
  [`CONTRIBUTING.md`](../CONTRIBUTING.md);
- [x] зафиксированные Rust- и npm-зависимости (`Cargo.lock` и
  `extensions/vscode/package-lock.json`);
- [x] MSRV Rust 1.88 в [`Cargo.toml`](../Cargo.toml) и отдельная проверка MSRV
  в CI;
- [x] `rustfmt`, Clippy и тесты на Windows и Linux;
- [x] coverage ratchet, Miri и fail-closed тесты Linux sandbox в
  [`.github/workflows/ci.yml`](../.github/workflows/ci.yml);
- [x] сборка release с `--locked`, архивы Windows/Linux и SHA-256 в
  [`.github/workflows/release.yml`](../.github/workflows/release.yml);
- [x] запрет `unsafe` в основном Rust crate;
- [x] локальная политика лицензий и advisories в [`deny.toml`](../deny.toml);
- [x] запрет inline API keys и fail-closed `access` в конфигурации агента;
- [x] игнорирование `.env`, локальных конфигураций, банков и основных каталогов
  запусков в [`.gitignore`](../.gitignore).

Эти пункты являются исходной базой, а не автоматическим разрешением на
публикацию.

## P0 — блокеры публикации

### 1. Очистить рабочее дерево и правила игнорирования

- [ ] Разобрать все изменённые и неотслеживаемые файлы; не использовать
  массовый `git add .` перед первым публичным коммитом.
- [ ] Не публиковать локальные отчёты
  `deepseek__deepseek-v4-flash.*.json`.
- [ ] Не публиковать сгенерированный каталог
  `workflows/swebench-verified/artifacts/`.
- [ ] Исключить `.cursor/plans/` либо очистить планы от локальных путей и
  внутреннего контекста.
- [ ] Убедиться, что `local/security-runs/`, `local/security-bank/` и host
  canary остаются локальными. Публичный `local/security-bank.example/`
  проверять отдельно как намеренно публикуемый red-team набор.
- [ ] Дополнить `.gitignore` для всех подтверждённых генерируемых артефактов и
  проверить правила через `git check-ignore`.

Проверка:

```powershell
git status --short
git ls-files --others --exclude-standard
git check-ignore -v -- `
  "deepseek__deepseek-v4-flash.regression-20260809-132621.json" `
  "workflows/swebench-verified/artifacts/_linux_init_probe" `
  ".cursor/plans"
```

Ожидаемый результат: каждый оставшийся untracked-файл либо намеренно войдёт в
публичный коммит, либо надёжно игнорируется.

### 2. Просканировать секреты в текущем дереве и полной истории

- [ ] Запустить специализированный scanner (например, Gitleaks) по рабочему
  дереву и всей истории, включая удалённые файлы и все refs.
- [ ] Вручную проверить конфигурации, eval-артефакты, логи, PEM-блоки, PAT,
  cloud credentials и provider API keys.
- [ ] Проверить, что примеры используют только `api_key_env` и безопасные
  placeholders.
- [ ] Если найден реальный секрет: сначала отозвать/ротировать его, затем
  очистить историю или создать новый чистый публичный репозиторий. Простого
  удаления файла в последнем коммите недостаточно.
- [ ] Сохранить отчёт scanner без самих секретов.

Пример проверки:

```powershell
gitleaks git . --redact --no-banner
rg -n -i `
  "(api[_-]?key|token|password|secret)\s*[:=]|BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|gh[pousr]_|AKIA" `
  . --glob "!.git/**" --glob "!target/**"
```

Ожидаемый результат: нет неразобранных findings; все false positives
задокументированы.

### 3. Удалить внутренние и персональные данные

- [ ] Заменить внутренние данные в
  [`crates/sandbox-linux/TESTING.md`](../crates/sandbox-linux/TESTING.md):
  `192.168.68.119`, `ubuntu-laptop`, `aidev`, путь к SSH identity и
  инструкции, привязанные к домашней лаборатории.
- [ ] Удалить абсолютные пользовательские пути, включая `Users/gazal`, из
  публикуемых файлов.
- [ ] Проверить документы, примеры, JSON-отчёты, логи и историю на IP-адреса,
  имена пользователей, внутренние hostnames и локальные каталоги.
- [ ] Расширить portability gate на публикуемые VS Code JSON-примеры и другие
  конфигурационные файлы, а не только `agent-config*.yaml`.

Проверка:

```powershell
rg -n -i `
  "192\.168\.|ubuntu-laptop|Users[/\\]gazal|id_ed25519|C:[/\\]Git" `
  . --glob "!.git/**" --glob "!target/**"
```

Ожидаемый результат: совпадения отсутствуют либо являются явно нейтральными
документированными примерами.

### 4. Опубликовать процесс сообщения об уязвимостях

- [ ] Добавить корневой `SECURITY.md`.
- [ ] Указать поддерживаемые версии, приватный канал связи, запрет публикации
  exploitable details в issue, сроки первого ответа и правила coordinated
  disclosure.
- [ ] Включить GitHub Private Vulnerability Reporting или указать рабочий
  security email.
- [ ] Сослаться на политику из корневого README и issue templates.
- [ ] Описать границы доверия: stdio/HTTP MCP, сетевой egress,
  `KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP`, protected store и ограничения OS
  sandbox.

Ожидаемый результат: внешний исследователь может сообщить об уязвимости, не
раскрывая её публично.

### 5. Подтвердить юридическую чистоту

- [ ] Подтвердить, что `Gazaliev Timur` в [`NOTICE`](../NOTICE) — правильный
  правообладатель, и обновить годы при необходимости.
- [ ] Убедиться, что на весь код, документацию, изображения, prompts и
  датасеты есть право публикации под Apache-2.0.
- [ ] Выполнить `cargo deny check` и отдельную проверку лицензий npm/Python
  зависимостей.
- [ ] Проверить лицензию и необходимую атрибуцию git submodule
  `.cursor/skills/rust-skills`.
- [ ] Решить, нужен ли `THIRD_PARTY_NOTICES` в исходниках или release-архивах.
- [ ] Подтвердить политику использования торговых марок `Kuibysheff` и
  `agent_Kuibysheff`.

Проверка:

```powershell
cargo deny check
git submodule status
```

Ожидаемый результат: нет неизвестного происхождения файлов или несовместимых
лицензий; необходимые notices включены.

### 6. Устранить путаницу имени, ссылок и версии

- [ ] Выбрать публичное имя и slug: `Agent-Kuibyshev` или
  `Agent-Kuibysheff`.
- [ ] Исправить ссылки на GitHub во всех README, release docs, badges и
  `extensions/vscode/package.json`.
- [ ] Проверить, что каждая публичная ссылка открывается без авторизации.
- [ ] Синхронизировать версию CLI `0.2.0`, release tag и примеры `v0.1.0` в
  [`RELEASING.md`](RELEASING.md).
- [ ] Документировать независимую версию VS Code extension `0.1.0` либо
  синхронизировать версии.
- [ ] Описать несовместимое изменение 0.2.0: обязательный `access` и migration
  path с `access: { mode: legacy }` только для осознанной совместимости.

Проверка:

```powershell
rg -n "Agent-Kuibyshev|Agent-Kuibysheff|0\.1\.0|0\.2\.0" `
  README.md docs extensions/vscode/package.json Cargo.toml
```

Ожидаемый результат: имя, URL, версии и инструкции не противоречат друг другу.

## P1 — публичная документация и управление

### 7. Подготовить onboarding для пользователей и контрибьюторов

- [ ] Расширить [`CONTRIBUTING.md`](../CONTRIBUTING.md): Rust 1.88+, Node 20,
  Python 3.12, Docker/WSL требования, установка hooks, команды проверок и
  правила PR.
- [ ] Добавить отдельные инструкции install, upgrade и uninstall для
  Windows/Linux, включая сохранение или удаление пользовательских данных.
- [ ] Документировать клонирование git submodule через
  `--recurse-submodules` или отказаться от обязательного submodule.
- [ ] Добавить `CHANGELOG.md` и migration guide либо включить эквивалентные
  разделы в release notes.
- [ ] Определить языковую стратегию для русскоязычных документов: перевод,
  двуязычность или явная маркировка.
- [ ] Решить, остаются ли публичными внутренние review/backlog документы в
  `docs/architecture-review/` и `docs/FURTHER_FIXES.md`.

Ожидаемый результат: новый пользователь может установить продукт, а новый
контрибьютор — воспроизвести проверки без приватных знаний.

### 8. Добавить минимальную инфраструктуру сообщества

- [ ] Добавить Code of Conduct либо явно объяснить принятое правило поведения.
- [ ] Добавить bug/feature/security issue templates.
- [ ] Добавить PR template с DCO, тестами, документацией и security-impact
  пунктами.
- [ ] Добавить `CODEOWNERS` для `src/`, sandbox, release и security-sensitive
  workflows.
- [ ] Опубликовать support policy: Issues, Discussions, best-effort или
  поддерживаемые SLA.
- [ ] Настроить label taxonomy и правила triage.

Ожидаемый результат: публичные обращения попадают в понятный и безопасный
процесс.

## P1 — качество, CI и supply chain

### 9. Сделать CI эквивалентным обязательным локальным gates

- [ ] Добавить `cargo deny check` в CI и pre-release checks.
- [ ] Добавить автоматический secret scan для push/PR и всей истории перед
  первым открытием.
- [ ] Запускать offline
  `workflows/security-sandbox/test_security_lib.py` без API key.
- [ ] Запускать `scripts/test_detached_workflows.py` и portability checks в
  кроссплатформенном виде.
- [ ] Разобрать различия `scripts/check.ps1` и `scripts/check.sh`, включая
  разный default для AoC regression.
- [ ] Добавить `npm audit` или эквивалентный контролируемый audit процесс для
  VS Code extension.
- [ ] Заменить placeholder fuzz/mutation gate на реальную проверку или не
  представлять его как действующий контроль.

Ожидаемый результат: обязательные проверки не зависят только от локального
pre-commit hook, который можно обойти.

### 10. Укрепить GitHub Actions и зависимости

- [ ] Указать минимальные `permissions` для каждого workflow/job.
- [ ] Pin security-critical third-party Actions на commit SHA и организовать
  обновление pins.
- [ ] Настроить Dependabot или Renovate для Cargo, npm и GitHub Actions.
- [ ] Проверить, что PR из forks не получает write permissions или release
  secrets.
- [ ] Проверить retention и содержимое загружаемых CI artifacts.
- [ ] Использовать `--locked` во всех release-like сборках, включая nightly
  release smoke.

Ожидаемый результат: CI не выдаёт лишние права и воспроизводимо использует
проверенные зависимости.

### 11. Зафиксировать поддерживаемые платформы

- [ ] Подтвердить поддержку Windows x86_64 и Linux x86_64.
- [ ] Явно указать статус macOS: unsupported, build-from-source или
  поддерживаемая платформа с CI.
- [ ] Не заявлять Linux aarch64 до появления сборки и тестов; Linux sandbox
  сейчас содержит x86_64-only seccomp.
- [ ] Зафиксировать glibc/дистрибутивный baseline для Linux binary.
- [ ] Проверить release archive и `--help` на чистых Windows/Linux машинах.

Ожидаемый результат: публичные обещания совпадают с CI, sandbox и release
matrix.

## P2 — релиз и дистрибуция

### 12. Усилить целостность релизов

- [ ] Описать проверку `.zip.sha256` в пользовательской документации.
- [ ] Добавить SBOM в SPDX или CycloneDX для каждого release.
- [ ] Добавить Sigstore/cosign или GitHub artifact attestations.
- [ ] Проверить воспроизводимость сборки и записать известные ограничения.
- [ ] Сформировать release-candidate checklist с opt-in AoC, SWE-bench и LLM
  security regression.

### 13. Определить распространение VS Code extension

- [ ] Выбрать VSIX-only, Visual Studio Marketplace, Open VSX или комбинацию.
- [ ] Проверять сборку `.vsix` в CI.
- [ ] Подтвердить владельца publisher `kuibysheff` и процесс публикации.
- [ ] Документировать Windows/PowerShell ограничения extension.
- [ ] Явно заявить telemetry policy.

### 14. Подготовить проектную страницу

- [ ] Обновить GitHub description, topics, homepage и social preview.
- [ ] Проверить README badges и ссылки в приватном/публичном режиме.
- [ ] Добавить понятные screenshots или короткий demo для основного сценария.
- [ ] Решить, включать ли GitHub Discussions.
- [ ] Подготовить краткое объявление с точными ограничениями безопасности и
  поддерживаемых платформ.

## День открытия

- [ ] Заморозить release candidate commit и убедиться, что рабочее дерево
  чистое.
- [ ] Повторить secret scan текущего дерева, полной истории и release archive.
- [ ] Убедиться, что все обязательные CI checks зелёные на точном commit.
- [ ] Установить release archive на чистой Windows и Linux машине; выполнить
  `--help`, безопасный smoke run и uninstall.
- [ ] Проверить checksums, SBOM/attestation, содержимое архивов и release notes.
- [ ] Включить branch protection/ruleset для основной ветки: reviews,
  обязательные checks, запрет force-push и удаления.
- [ ] Включить Private Vulnerability Reporting, Dependabot alerts и secret
  scanning, если они доступны.
- [ ] Проверить GitHub metadata, License detection, SECURITY link, issue
  templates и публичные ссылки.
- [ ] Выполнить финальный Go/No-Go по критериям ниже.

## После открытия

- [ ] В первые 24 часа проверить установку по публичным URL и доступность
  release artifacts без прав владельца.
- [ ] Разобрать первые issues/security alerts и удалить ошибочно опубликованные
  artifacts; при утечке немедленно ротировать секреты.
- [ ] Через неделю пересмотреть onboarding, support policy и частые вопросы.
- [ ] Ежемесячно проверять dependencies, advisories и supported versions.
- [ ] Перед каждым тегом повторять release-candidate и Go/No-Go проверки.

## Журнал принятых исключений

Для каждого незакрытого `P1` перед публикацией записать:

- пункт и владелец решения;
- причина принятия риска;
- влияние на пользователей и безопасность;
- временная компенсация;
- крайний срок и issue для закрытия.

`P0` нельзя перевести в исключение без изменения этого документа и явно
зафиксированного решения владельца проекта.

## Go/No-Go

Публичный релиз получает **Go**, только если одновременно выполнено всё:

- все `P0` закрыты и подтверждены повторной проверкой на release candidate;
- нет неизвестных секретов, персональных или внутренних данных в дереве,
  истории и release archive;
- лицензии, правообладатель, торговые марки и сторонние атрибуции подтверждены;
- CI зелёный, release устанавливается на заявленных платформах, а checksum
  проверяется;
- `SECURITY.md` содержит рабочий приватный канал сообщения;
- все незакрытые `P1` записаны в журнале исключений с владельцем и сроком.

Если хотя бы одно условие не выполнено, решение — **No-Go**.
