# Further Fixes Plan

Roadmap for remaining rust-skills improvements for `agent_Kuibysheff`.

**Last updated:** 2026-07-11

---

## Completed (baseline)

These items were implemented in the initial rust-skills remediation:

- [x] MCP actor pattern — no `Mutex` held across `.await` on RPC path
- [x] JSONL logger background writer — no lock across file I/O
- [x] Message history count pruning (`MAX_TAIL_MESSAGES = 30`)
- [x] Blocking startup I/O moved to `spawn_blocking` in `main.rs`
- [x] `tracing` + `tracing-subscriber` instrumentation
- [x] `allowed_tools: Option<HashSet<String>>` semantics (`None` = unrestricted)
- [x] Release profile + `rust-version = "1.80"` in `Cargo.toml`
- [x] MCP stderr piped (was discarded)
- [x] `Ordering::Relaxed` for MCP request IDs

---

## Phase 1 — High impact, low risk (next sprint)

### 1.1 Reduce clones in the agent hot path

**Rules:** `own-borrow-over-clone`, `anti-clone-excessive`, `mem-avoid-format`

| Location | Issue | Fix |
|----------|-------|-----|
| `src/provider/openai_compat.rs` | `messages.to_vec()` on every completion | Serialize from `&[ChatMessage]` directly, or reuse a request buffer |
| `src/agent/loop.rs` | `tool_call.arguments.clone()` for call + log | Log after call using moved args, or borrow where possible |
| `src/main.rs` | Large `format!` system prompt every run | Build with `write!` into a pre-sized `String`, or template cache keyed by settings hash |

**Acceptance:** Profile one 10-iteration run; per-request allocations should drop measurably.

---

### 1.2 Decouple trait error types from implementations

**Rules:** `api-sealed-trait`, `err-custom-type`, `trait-associated-type-vs-generic`

Current coupling:

- `ModelClient` → `openai_compat::ProviderError`
- `ToolExecutor` → `stdio_client::McpError`
- `AgentError::Logging(String)` is stringly-typed

**Plan:**

1. Add `provider::Error` and `mcp::Error` enums at module boundaries.
2. Implement `From<ProviderError>` / `From<McpError>` / `From<LoggingError>` into `AgentError`.
3. Update trait signatures to return boundary errors.

**Acceptance:** `src/agent/loop.rs` no longer imports `openai_compat` or `stdio_client` for error types.

---

### 1.3 Character-aware history pruning

**Rules:** `mem-*`, `perf-profile-first`

Current pruning caps message *count* only. Long tool results can still blow token budgets.

**Plan:**

1. Add `MAX_HISTORY_CHARS` (e.g. 200_000) alongside `MAX_TAIL_MESSAGES`.
2. Prune by total char count of non-system messages, not just count.
3. Add unit test with oversized tool result payloads.

**Acceptance:** A synthetic 100k-char tool result does not leave history unbounded.

---

### 1.4 Policy enforcement integration test

**Rules:** `test-integration-dir`, `api-parse-dont-validate`

`allowed_tools: Option<HashSet>` is implemented but not covered end-to-end.

**Plan:**

- Test: model calls `home.write` with `Some({"home.read"})` → agent rejects, retries, eventually succeeds or stops.
- Test: `None` allows any tool (regression guard).

---

## Phase 2 — Architecture improvements (medium effort)

### 2.1 Remove `async_trait` via generics

**Rules:** `async-fn-in-trait`, `anti-type-erasure`

`Arc<dyn ModelClient>` forces `async_trait`. Migration path:

```rust
pub struct AgentEngine<M, T> {
    model: M,
    tools: T,
    loggers: Loggers,
}
where
    M: ModelClient + Send + Sync,
    T: ToolExecutor + Send + Sync,
```

**Steps:**

1. Make `AgentEngine` generic over `M` and `T`.
2. Keep test fakes as concrete types (no `dyn`).
3. In `main`, use concrete `OpenAiCompatClient` + `CompositeToolExecutor` (monomorphized).
4. Drop `async-trait` dependency.

**Trade-off:** Slightly larger binary; cleaner async, no boxed futures.

---

### 2.2 Fully async config/settings/context loaders

**Rules:** `async-tokio-fs`

`spawn_blocking` works but maintains a sync/async split.

**Plan:**

1. Add `load_config_async`, `load_settings_async`, `build_input_files_context_async` using `tokio::fs`.
2. Keep sync wrappers for unit tests (thin `std::fs` shims).
3. Remove `spawn_blocking` from `main.rs`.

---

### 2.3 Graceful shutdown and cancellation

**Rules:** `async-cancellation-token`, `async-cancel-safety`

**Status:** done (architecture-review/07) — `RunCancel` from composition root into
agent loop / MCP actors / `home.run` budget clamp; wall-clock expiry →
`stop_reason: limit_reached`. External Ctrl-C wiring still optional follow-up.

---

### 2.4 MCP robustness

**Rules:** `obs-error-chain`, `err-context-chain`

| Gap | Fix |
|-----|-----|
| stderr piped but not consumed | Tee stderr to MCP log file or tracing |
| Actor channel fixed capacity (32) | Backpressure + timeout on enqueue |
| No reconnect on child exit | Detect EOF, surface `McpError::ActorClosed` with child exit code |
| `wait_for_response` ignores out-of-order notifications | Log unmatched messages at `debug` |

---

## Phase 3 — API quality and maintainability

### 3.1 Public API documentation

**Rules:** `doc-all-public`, `doc-errors-section`

Undocumented public items:

- `AgentEngine`, `AgentRunRequest`, `RunOutput` fields
- `CompositeToolExecutor`
- `SkillsCatalog` helpers beyond `parse`

**Plan:** Add `///` docs + `# Errors` to all `pub` items; add 2–3 doctests on `SkillsCatalog` and `HomeFs`.

---

### 3.2 Skills DSL parser upgrade

**Rules:** `api-parse-dont-validate`, `anti-stringly-typed`

Current regex-based parser is brittle (nested braces, escaped quotes).

**Plan:**

1. Define AST types (`SkillBlock`, `Policy`, `ToolList`).
2. Hand-written parser or `winnow`/`nom` for the small grammar.
3. Preserve existing valid DSL; add error spans with line/column.
4. Keep regex parser tests as regression; add malformed-input tests.

---

### 3.3 Project hygiene

**Rules:** `proj-msrv-declare`, `name-crate-no-rs`, edition guide

| Item | Action |
|------|--------|
| Crate name `agent_Kuibysheff` | Rename to `agent-kuibysheff` (breaking; coordinate with consumers) |
| Edition 2021 → 2024 | Evaluate `unsafe_op_in_unsafe_fn`, `unsafe` attribute changes |
| CI | Add `cargo fmt --check`, `cargo clippy -- -D warnings`, MSRV pin job |
| README | Document `RUST_LOG`, `allowed_tools` semantics, limits |

---

## Phase 4 — Security and edge cases

### 4.1 `HomeFs` hardening (Windows focus)

**Rules:** `type-newtype-validated`

| Risk | Mitigation |
|------|------------|
| `starts_with` on canonical paths (Windows prefix edge cases) | Normalize prefix after canonicalization on both paths |
| Symlink escape on write | Add integration test with symlink inside home pointing outside |
| UNC paths (`\\?\`) | Reject `Prefix` components (already done); add Windows-specific test |

---

### 4.2 Secrets and logging

**Rules:** `obs-no-sensitive-data`

**Plan:**

1. [x] Audit JSONL payloads — redact sensitive keys and truncate long strings via `logging.audit_redaction` on EventSink (chat history / ACP / semantic PII still open).
2. [x] Provider `#[instrument(skip(self, …))]` so `api_key` on the client is not recorded in span fields.
3. Never log full model responses when they contain user PII (chat history / deeper PII heuristics — open).

---

## Phase 5 — Testing and performance (when needed)

### 5.1 Test coverage gaps

**Rules:** `test-*`

| Test | Purpose |
|------|---------|
| Provider retry on 429/5xx | Mock HTTP server |
| MCP timeout | Slow/fake MCP server |
| Token limit mid-tool-loop | Verify clean stop |
| `prune_message_history` under char budget | After Phase 1.3 |
| Property tests (`proptest`) | Path validation in `HomeFs`, config validation |

### 5.2 Benchmarks

**Rules:** `test-criterion-bench`, `perf-profile-first`

Only after Phase 1.1 — benchmark:

- Agent loop iteration (fake model)
- `HomeFs::read` / `write`
- Skills DSL parse

---

## Execution order

```
1.1 Reduce hot-path clones
 └─> 1.2 Boundary error types
      └─> 1.3 Char-aware pruning
           └─> 1.4 Policy integration tests
                └─> 2.1 Generic AgentEngine / drop async_trait
                     └─> 2.2 Async loaders
                          └─> 2.3 CancellationToken
                               └─> 2.4 MCP robustness
                                    └─> 3.x API docs + DSL parser
                                         └─> 4.x Security hardening
                                              └─> 5.x Benchmarks + proptest
```

---

## Effort estimates

| Phase | Items | Effort | Value |
|-------|-------|--------|-------|
| 1 | Clones, errors, char prune, tests | 1–2 days | High |
| 2 | Generics, async loaders, cancel, MCP | 3–5 days | High |
| 3 | Docs, DSL parser, rename | 2–3 days | Medium |
| 4 | HomeFs Windows, log redaction | 1–2 days | Medium–High |
| 5 | Benchmarks, proptest | 1–2 days | Low until scale matters |

---

## Recommended next PR

**Title:** Agent hot-path memory + error boundaries

1. Remove `messages.to_vec()` clone in provider
2. Introduce `provider::Error` / `mcp::Error`
3. Add char-aware pruning
4. Add `allowed_tools` policy integration test

This closes the remaining CRITICAL/HIGH gaps from the original rust-skills review without a large architectural change.

---

## References

- Rust skills guide: `.cursor/skills/rust-skills/SKILL.md`
- Original review: chat session 2026-07-11
