# 18 — Composition-root hygiene in `app`

**Status:** open
**Severity:** P2
**Area:** `src/app.rs`, `src/lib.rs`
**Rust-skills:** `proj-lib-main-split`, `api-builder-pattern`, `api-non-exhaustive`, `test-cfg-test-module`
**Review:** [review-2026-09.md](review-2026-09.md)

## Problem

`src/app.rs` is the CLI composition root (~546 lines) and is `pub` so `main.rs` can call it. Three subcommands each build the same Tokio runtime. The prompt-args bag is fifteen public fields with no constructor. The file reaches past the MCP facade into `stdio_client`. Nothing in the file is unit-tested, including billing source order and the process exit code.

This was suggestion 1 in [rust-review-findings.md](../rust-review-findings.md) (2026-08-04) and is still open.

## Evidence

Identical runtime construction in `run_worker` (`src/app.rs` ~192), `run_acp` (~232), and `run_a2a` (~254): `Builder::new_multi_thread().enable_all().build()`.

```rust
// src/app.rs
use crate::mcp::stdio_client::McpRegistry;
```

`lib.rs` documents `mcp` as `McpRegistry` and `McpError`, not the stdio module. `McpRegistry` is re-exported from `crate::mcp`. `src/commands/check.rs` has the same deep import; fix it in the same PR if it is a one-line change.

```rust
// src/app.rs — fifteen public fields, no constructor
pub struct AgentPromptArgs {
    pub config: PathBuf,
    pub settings_dir: PathBuf,
    pub home: PathBuf,
    // ...
    pub events: AgentEventTx,
}
```

`app` is not in the stable-surface list at the top of `src/lib.rs`, but `pub mod app` still exports `run_agent_prompt` and `AgentPromptArgs` to downstream crates.

## Acceptance

- [ ] One `build_runtime()` (private) used by `run`, `acp`, and `a2a`. Failure message stays `failed to start tokio runtime`.
- [ ] `app.rs` imports `McpRegistry` from `crate::mcp`.
- [ ] `AgentPromptArgs` is constructed in one place. Either `#[non_exhaustive]` plus a builder/constructor, or `app` is `#[doc(hidden)]` and the struct is `pub(crate)` if no external crate needs it. Check `extensions/` and tests before narrowing visibility; ACP and A2A are in-crate.
- [ ] Unit tests for `exit_code_for_run_output` (error vs goal vs limit) and for `build_billing_resolver` source order (provider, then mcp, then catalog, skipping missing sources).
- [ ] No new public items on the stable facade.

## Suggested approach

1. Extract `build_runtime() -> Result<Runtime, io::Error>` and map the error at each call site so stderr text does not drift.
2. Switch the import. `McpRegistry::connect_all_isolated` must remain visible through `crate::mcp` (re-export if it is only on the stdio module today).
3. Decide visibility from a grep of `agent_Kuibysheff::app` outside `src/`. In-crate callers (`acp`, `a2a`) can use `pub(crate)`.
4. Add `#[cfg(test)]` tests beside `exit_code_for_run_output`. For the resolver, pass a config with a fixed `source_order` and assert the chain length and resolver names without a live MCP server.

## Related

- [10](10-curate-public-api.md) — facade rules this item follows.
- [15](15-newtype-ids-and-event-kinds.md) — ids on `AgentPromptArgs`.
- [rust-review-findings.md](../rust-review-findings.md) suggestion 1.
