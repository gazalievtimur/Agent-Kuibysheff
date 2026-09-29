# 16 — `deny_unknown_fields` on every config DTO

**Status:** open
**Severity:** P2
**Area:** `src/config/mod.rs`, `src/limits.rs`
**Rust-skills:** `serde-deny-unknown-fields`, `serde-default-compat`
**Review:** [review-2026-09.md](review-2026-09.md)

## Problem

`AppConfig` and the billing and MCP DTOs reject unknown keys. Nested provider, logging, and limits structs do not. A typo in those sections is dropped on load, so a mis-set timeout, sink, or token cap stays at the default with no error.

`#[serde(deny_unknown_fields)]` on a parent does not apply to fields of a child struct. Unknown keys inside `provider`, `logging`, and `limits` are ignored today.

## Evidence

Denied (examples): `AppConfig` (`src/config/mod.rs:59`), `BillingConfig` (`:80`), MCP server DTOs (`:348` and following).

Not denied:

| Struct | Declaration |
|--------|-------------|
| `ProviderConfig` | `src/config/mod.rs:201` |
| `LoggingConfig` | `src/config/mod.rs:535` |
| `AuditRedactionConfig` | `src/config/mod.rs:561` |
| `LimitsConfig` | `src/limits.rs:8` |

`LogSinkConfig` is an internally tagged enum (`tag = "type"`). Confirm whether unknown fields on each variant are already rejected by the tag, and add `deny_unknown_fields` on variants if they are not.

## Acceptance

- [ ] `ProviderConfig`, `LoggingConfig`, `AuditRedactionConfig`, and `LimitsConfig` use `#[serde(deny_unknown_fields)]`.
- [ ] Fixture configs still load: `agent-config.example.yaml`, `agent-config.local*.yaml`, `src/templates/agent_init/*`, and config tests.
- [ ] A unit test feeds one unknown key in `provider`, `logging`, and `limits` and expects `ConfigError` (or the limits deserializer error mapped into it).
- [ ] Optional keys that are absent keep their `#[serde(default)]` behavior. Deny-unknown and default are compatible; do not drop defaults.

## Suggested approach

1. Add the attribute to the four structs.
2. Run the config test suite and fix any fixture that relied on a stray key (rename the key or delete it).
3. Add the negative tests next to the existing `AppConfig` deny-unknown test if one exists; otherwise add them under `config/mod.rs` `#[cfg(test)]`.

## Related

- [05](05-break-config-access-cycle.md) — access DTOs already live in `access::config`; check they deny unknown fields too while touching this, and fold a miss into the same PR.
- [14](14-break-remaining-module-cycles.md) — if logging DTOs move to `logging::config`, apply the attribute in that move instead of twice.
