# 05 — Разорвать цикл config ↔ access

**Status:** done  
**Severity:** P1  
**Area:** `src/config.rs`, `src/access/`  
**Rust-skills:** `api-parse-dont-validate`, `proj-mod-by-feature`, `conv-tryfrom-fallible`

## Problem

- `config` вызывает validation/resolution из `access`
- `access` импортирует raw DTO из `config`

DTO и compiled policy переплетены — сложнее тестировать resolve отдельно от YAML schema.

## Evidence

- `src/config.rs` — imports access validation
- `src/access/mod.rs` — imports config DTOs (`AccessPolicyConfig`, …)

## Acceptance

- [x] Raw access DTOs живут рядом с config (или в `access::raw`) без зависимости от resolve
- [x] `ResolvedAccessPolicy::try_from` / `compile` принимает borrowed DTO, не тянет clap/CLI
- [x] Unit-тесты resolve без полного `load_config`
- [x] Нет взаимных `use crate::{access, config}` циклов на уровне типов

## Suggested approach

1. Вынести `AccessPolicyConfig` (+ nested) в `access::config` или оставить в config, но access принимает `&AccessPolicyConfig` через публичный API без обратного импорта сложных config types.
2. Лучший вариант: `access` владеет DTO + resolve; `config` реэкспортирует или содержит `access: Option<AccessPolicyConfig>`.
3. CLI overrides остаются в composition root / config apply, не в access.

## Resolution

- `src/access/config.rs` owns raw DTOs (`AccessPolicyConfig` and nested types).
- `access` validates/resolves via `AccessError`; no `crate::config` import.
- `ResolvedAccessPolicy: TryFrom<AccessResolveInput<'_>>` compiles borrowed DTOs + `config_dir`.
- `config` embeds `Option<AccessPolicyConfig>`, maps `AccessError` → `ConfigError`, and re-exports DTO names for compatibility.
- CLI overrides stay in `config::apply_cli_overrides` (composition root).

## Notes

Не смешивать с [08](08-exit-code-on-run-error.md) — это про границы модулей, не CLI semantics.
