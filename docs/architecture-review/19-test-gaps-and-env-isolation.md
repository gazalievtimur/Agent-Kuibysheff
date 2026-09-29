# 19 — Test gaps and environment isolation

**Status:** open
**Severity:** P3
**Area:** `src/commands/config/`, `src/a2a/auth.rs`, `src/logging/paths.rs`, `tests/logging_integration.rs`, `crates/sandbox-windows`, `crates/sandbox-linux`, `src/agent/run_cancel.rs`, `src/mcp/stdio_client.rs`, `src/agent/loop/engine.rs`
**Rust-skills:** `test-cfg-test-module`, `test-fixture-raii`, `err-expect-bugs-only`, `async-cancel-safety`
**Review:** [review-2026-09.md](review-2026-09.md)

## Problem

A few large modules have no unit tests. Several tests mutate process environment variables; on Windows CI, `cargo test` does not pass `--test-threads=1`, so those tests can race. Two production `.expect` calls encode a "we just pushed" invariant the type system does not hold. Two spawned tasks and the engine `select!` arms are intentional but easy to "fix" into a deadlock or a dropped side effect if the comment is missing.

## Evidence

**No `#[cfg(test)]` module:**

| File | Notes |
|------|--------|
| `src/commands/config/mcp.rs` | MCP config CRUD |
| `crates/sandbox-windows/src/native/process.rs` | ~27 KB, Win32 process launch, `unsafe` |
| `crates/sandbox-linux/src/native/mount.rs` | pivot_root / bind mounts |
| `crates/sandbox-linux/src/native/probe.rs` | sandbox capability probe |

**Process-global env in tests:**

| Site | What it sets |
|------|----------------|
| `src/a2a/auth.rs` (~117–133) | `A2A_TEST_TOKEN`, `A2A_TEST_TOKEN_EMPTY` via `set_var` |
| `src/logging/paths.rs` (~137–138) | `HOME`, `USERPROFILE` |
| `tests/logging_integration.rs` (~14–23) | `EnvRestore` puts the old value back on drop, but two tests can still overlap |

**`.expect` after `push`:**

- `src/commands/config/mcp.rs:85` — `profile.config.mcp.last().expect("just pushed")`
- `src/commands/config/skill.rs:61` — `catalog.skills.last().expect("just pushed")`

**Intentional concurrency (document, do not delete):**

- `RunCancel::arm_deadline` (`src/agent/run_cancel.rs:57`) spawns a sleeper and drops the `JoinHandle`. A generation counter ignores a stale sleeper. The task is detached on purpose.
- `spawn_stderr_drain` (`src/mcp/stdio_client.rs:360`) spawns a reader and drops the handle so a full stderr pipe cannot stall the child.
- `src/agent/loop/engine.rs` `select!` around `complete_accounted` (~225) and `call_tool` (~508) is not cancel-safe: dropping the future abandons the HTTP call or the tool. That matches `RunCancel` and item [07](07-hard-deadline-cancellation.md). The cancel arm must keep emitting `ToolFinish`.

`unreachable!` in `src/commands/config/import.rs:353` is inside `#[cfg(test)]`. Out of scope.

## Acceptance

- [ ] `commands/config/mcp.rs` has unit tests for add/remove (or the shared helper they call), using a temp profile.
- [ ] Env-mutating tests take a process-wide `Mutex` (or `temp-env`) so parallel `cargo test` on Windows cannot interleave `set_var`. `EnvRestore` stays as the drop guard inside the lock.
- [ ] Both `"just pushed"` expects are gone. Use the value returned by the insert, or `last_mut` immediately after `push` without `expect` if the push is in the same block and the compiler can see it — prefer returning the inserted element from a helper.
- [ ] Module docs on `RunCancel::arm_deadline`, `spawn_stderr_drain`, and the two engine `select!` sites state why the handle is dropped and why the future is not cancel-safe.
- [ ] Sandbox `process.rs` / `mount.rs` / `probe.rs`: at least one test each for a pure helper (path, ACL string, probe result parsing) that does not need a real namespace or AppContainer. Skip with a reason if a function cannot be tested without the OS feature; do not add a test that always no-ops.

## Suggested approach

1. Serialize env tests first; that is the only flake risk.
2. Replace the two expects while adding the mcp command test.
3. Add the doc comments in the same PR as any edit to those functions, or as a docs-only commit if no behavior change is needed.
4. Sandbox helper tests are last and may split by OS.

## Related

- [07](07-hard-deadline-cancellation.md) — cancel-safety contract.
- [17](17-decompose-engine-run-inner.md) — do not lose the `select!` comments when extracting `RunState`.
- [18](18-app-composition-root-hygiene.md) — `app.rs` tests live there, not here.
