# 12 — Legacy access mode (без `access`)

**Status:** done  
**Severity:** P2  
**Area:** `src/access/`, `CONTRACT.md`, orchestrator workflows  
**Rust-skills:** `api-parse-dont-validate`, `type-enum-states`, `obs-structured-fields`  
**Branch:** (workspace; option B)

## Problem

Если блок `access` отсутствует — legacy mode: permissive home/workspace/input семантика, `home.run` скрыт. Удобно для демо, опасно как тихий default в проде: omission = wide FS capability.

## Evidence

- CONTRACT: таблица legacy vs strict
- `ResolvedAccessPolicy` / `HomeFsPolicy::from_access` legacy branches

## Acceptance

Выбрано направление **B. Migrate to strict-by-default**:

- [x] Breaking: требовать `access` или явный `access: { mode: legacy }`
- [x] Migration guide + bump `0.2.0` / CONTRACT

(A и C не выбраны.)

## Implementation notes

- `AccessModeField` (`strict` | `legacy`) on `AccessPolicyConfig`; default `strict` when `access` present.
- Omitted `access` → `AccessError` / config validation failure.
- `access.mode: legacy` → `ResolvedAccessPolicy::legacy()`; mixed grants rejected.
- Init templates and examples ship minimal strict `access`; docs updated.

## Notes

Не путать с sandbox availability: legacy всё равно не рекламирует `home.run`.
