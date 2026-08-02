# 08 — Non-zero exit при stop_reason=error

**Status:** done  
**Severity:** P1  
**Area:** `src/main.rs`, `CONTRACT.md`  
**Rust-skills:** `type-enum-states`, `doc-all-public`  
**Branch:** `feat/exit-code-on-run-error`

## Problem

`run_worker` печатает JSON и возвращает `ExitCode::SUCCESS`, даже если `stop_reason == "error"`. Orchestrator, смотрящий только на exit code, считает run успешным.

Сейчас контракт говорит: ошибка выражается в JSON. Это валидно, но легко промахнуться при интеграции.

## Evidence

- `src/main.rs` — `run_worker`: serialize ok → SUCCESS
- `CONTRACT.md` — один JSON; failure = `stop_reason: error`

## Acceptance

Выбрать и задокументировать одну политику:

**Вариант A (рекомендуемый):**  
- exit ≠ 0 при `stop_reason == error` (и опционально при serialize fail)  
- JSON по-прежнему на stdout  

**Вариант B:**  
- exit всегда 0 при валидном JSON  
- явно в CONTRACT: «ignore exit code; parse stop_reason»  
- добавить `--fail-on-error` flag для A-поведения

- [x] CONTRACT + README согласованы
- [x] Тест/скрипт check: error output → ожидаемый exit
- [x] Не ломать management commands (`init`/`check`) — у них своя семантика

## Implementation notes

- Выбран **вариант A**.
- `exit_code_for_run_output`: `Error` → `FAILURE`; `GoalReached` / `LimitReached` → `SUCCESS`.
- Serialize fail по-прежнему печатает minimal error JSON и `FAILURE`.
- `tests/run_exit_code.rs`: missing config → JSON + non-zero; `check` остаётся human-text + own exit.

## Suggested approach

1. Согласовать с потребителями (1c-dev scripts, CI).
2. Минимальный патч в `run_worker` + CONTRACT bullet.
3. Обновить `scripts/check*.ps1/sh` / workflow examples при необходимости.
