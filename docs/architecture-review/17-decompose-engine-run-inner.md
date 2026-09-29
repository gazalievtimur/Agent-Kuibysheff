# 17 — Decompose `AgentEngine::run_inner`

**Status:** open
**Severity:** P2
**Area:** `src/agent/loop/engine.rs`; continues [11](11-split-god-modules.md)
**Rust-skills:** `proj-mod-by-feature`, `mem-box-large-variant`, `anti-clone-excessive`, `perf-profile-first`
**Review:** [review-2026-09.md](review-2026-09.md)

## Problem

`AgentEngine::run_inner` is one function of about 610 lines (`src/agent/loop/engine.rs`, from the `async fn run_inner` at line 121 through the end of the iteration loop). It carries `#[allow(clippy::too_many_lines)]` and `#[allow(clippy::result_large_err)]` because the error type is `(AgentError, UsageReport)`.

The failure epilogue — persist chat history, build a usage report, return `Err` — is copied at each pipeline and provider failure (six sites in the first half of the function alone). A change to that epilogue has to be repeated by hand.

`engine.rs` is about 1450 lines. Roughly the second half is tests, which is fine, but the production function is still the review bottleneck [11](11-split-god-modules.md) left after splitting `directive` and `history`.

## Evidence

```rust
// src/agent/loop/engine.rs
#[instrument(skip(self, request), fields(prompt_len = request.prompt.len()))]
#[allow(clippy::too_many_lines)]
// AgentError + UsageReport exceeds the clippy large-Err threshold; kept inline for call-site clarity.
#[allow(clippy::result_large_err)]
async fn run_inner(
    &self,
    request: AgentRunRequest,
) -> Result<RunOutput, (AgentError, UsageReport)> {
```

Hot-path clones to measure before removing (do not delete on suspicion):

| Clone | Line | Why it exists |
|-------|------|----------------|
| `messages.clone()` into `full_history` | ~141 | pruned `messages` and full transcript diverge |
| `arguments.clone()` into `ToolStart` | ~505 | the same `Value` is moved into `call_tool` |
| `tool_response.clone()` into `ToolFinish` | ~609 | the same `Value` is also embedded in the next user message |

## Acceptance

- [ ] `run_inner` no longer needs `too_many_lines` or `result_large_err`.
- [ ] A `RunState` (messages, full history, metrics, cost tracker, final result, stop reason, diagnostics) owns `fail`, tool-call execution, and directive parse failure. The persist-and-report epilogue exists once.
- [ ] Error return is `Result<RunOutput, Box<RunFailure>>` (or an equivalent small type) so the large tuple is not the `Err` variant.
- [ ] Behavior tests in `engine.rs` stay green: limits, cancel during tool call, parse failure, billing, pipeline stages.
- [ ] Any clone removal is backed by a measurement on a multi-iteration fake-model run, or is obviously a move of a value that is not used again. No speculative micro-opts.

## Suggested approach

1. Introduce `RunState` in `engine.rs` (or `agent/loop/state.rs` if the file shrinks enough to justify it). Methods borrow `&self` engine pieces they need (`loggers`, `tools`, `pipeline_events`) as arguments so the state struct stays data.
2. Replace each copied epilogue with `state.fail(err).await`.
3. Box the error. Drop both `allow` attributes.
4. Profile the three clones. Prefer moving `arguments` into the event after the call when the event can take ownership, and `Arc<str>`-style sharing only if the profile says the `Value` clone dominates.
5. Refresh the file-size table in [11](11-split-god-modules.md) when a split lands. Current sizes to beat are listed there.

## Related

- [11](11-split-god-modules.md) — `config`, MCP HTTP/stdio, OAuth, billing, provider, and `access` are still over 1000 lines.
- [07](07-hard-deadline-cancellation.md) — cancel arms stay in the extracted tool-call path; do not drop the `ToolFinish` on cancel (`engine.rs` around line 522).
- [19](19-test-gaps-and-env-isolation.md) — document the intentional non-cancel-safe `select!` arms while touching them.
