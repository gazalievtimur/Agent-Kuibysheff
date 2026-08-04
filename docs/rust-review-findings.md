# Rust Review Findings

Date: 2026-08-04

## Scope and Criteria

- Reviewed files in scope:
  - `src/**/*.rs`
  - `crates/**/src/**/*.rs`
- Excluded from scope:
  - `**/tests/**`
  - `**/*fixture*.rs`
  - `src/bin/mcp_stdio_fixture.rs`
  - `src/bin/sandbox_e2e_fixture.rs`
  - `crates/sandbox-windows/src/bin/sandbox_fixture.rs`
- Total reviewed candidates in scope: 82 files.
- Review protocol: `.cursor/rules/rust-review.mdc`.
- Style baseline: Rust Style Guide and `rustfmt.toml` (`edition = "2021"`, `max_width = 100`).
- CI gate baseline: `.github/workflows/ci.yml`.

## Hard Gates Result

- `cargo fmt --all -- --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — pass.

Blocker status from automated gates: no gate failures.

## Blockers

No blockers found in reviewed scope.

## Warnings

1. `src/context.rs`  
   `format_file` reads whole file with `read_to_string` before truncating to 50k chars; large allowed inputs can still spike memory.  
   Rule mapping: project-specific constraints (robustness under large inputs).

2. `src/agent/loop/engine.rs`  
   Cancellation paths set the same user-facing message as duration limit (`max_duration_sec`), including explicit user cancel; stop reason text is misleading.  
   Rule mapping: review outcome policy (correctness/UX).

3. `src/agent/loop/engine.rs`  
   If cancellation happens during tool call select branch, `ToolStart` can be emitted without matching `ToolFinish`, leaving dangling in-progress state for ACP consumers.  
   Rule mapping: review outcome policy (correctness for event stream consumers).

4. `src/agent/loop/history.rs`  
   Char-budget pruning drops oldest middle messages entirely; oversized single turn can be removed wholesale, reducing traceability of tool results.  
   Rule mapping: project-specific constraints (history safety/maintainability).

5. `src/commands/check.rs`  
   `spawn_blocking` join failure is mapped as `ConfigError::Validation`, conflating runtime task failure with config validation error.  
   Rule mapping: style checklist (error clarity).

6. `src/tools/fs_home.rs`  
   Rustdoc `# Errors` references `crate::mcp::Error`, but API returns `HomeFsError`.  
   Rule mapping: style checklist (docs consistency).

7. `src/tools/local_tools.rs`  
   Rustdoc `# Errors` references `crate::mcp::Error`, but API returns `LocalToolsError`.  
   Rule mapping: style checklist (docs consistency).

8. `src/access/mod.rs`, `src/sandbox/mod.rs`  
   Forbidden inherited env key list is duplicated in two modules, creating drift risk between policy validation and runtime enforcement.  
   Rule mapping: review outcome policy (maintainability).

9. `src/acp/server.rs`  
   Sessions are inserted but not removed; long-lived ACP process may accumulate session state.  
   Rule mapping: review outcome policy (maintainability).

10. `src/tools/local_tools.rs`  
    `resolve_existing_file` validates `is_file()` on symlink path but not on regular path branch; directory misuses degrade to generic I/O errors.  
    Rule mapping: style checklist (path handling clarity).

11. `crates/sandbox-windows/src/native/process.rs`  
    `read_pipe_bounded` stops reading when internal byte budget is exceeded and does not drain remaining output; can increase deadlock risk on verbose child output.  
    Rule mapping: review outcome policy (correctness/resilience).

12. `crates/sandbox-linux/src/native/probe.rs`  
    Probe validates namespace + mount primitives but not `clone3`/pidfd path used by runtime sandbox flow.  
    Rule mapping: project-specific constraints (probe/runtime parity).

13. `crates/sandbox-linux/src/native/caps.rs`  
    Hard-coded `CAP_LAST_CAP = 40` can become stale on newer kernels and miss dropping newer bounding capabilities.  
    Rule mapping: project-specific constraints (safety hardening).

14. `crates/sandbox-linux/src/native/helper.rs`  
    Timeout-path `poll` call lacks adjacent `// SAFETY:` note while project policy expects explicit unsafe justification.  
    Rule mapping: project-specific constraints (unsafe documentation discipline).

15. `crates/sandbox-windows/src/native/process.rs`  
    Adjacent DLL staging uses best-effort `let _ = std::fs::copy(...)` and ignores copy failures, which can hide setup issues behind later runtime failures.  
    Rule mapping: review outcome policy (diagnostic quality/fail-closed behavior).

## Suggestions

1. `src/app.rs`  
   Extract shared Tokio runtime builder used by both worker and ACP entry paths to reduce drift.

2. `src/agent/run_cancel.rs`  
   Document re-arm assumptions (single run lifecycle) or guard against future multi-arm task accumulation.

3. `src/agent/loop/directive.rs`  
   Consider optional tolerant mode for extracting first JSON object when model adds accidental preamble text.

4. `src/config.rs`  
   Tighten validation for HTTP MCP auth blocks with missing client identity metadata.

5. `src/tools/mod.rs`  
   Tool-allowed logs at `info` for every call can be noisy; consider `debug` or bounded logging strategy.

6. `src/skills/dsl.rs`  
   Regex-only DSL parsing is fragile for nested structures; consider parser hardening if DSL expands.

7. `src/logging/sink.rs`  
   Bounded channel backpressure can delay producers on slow disk; consider policy for bounded drops/metrics.

8. `src/tools/fs_home.rs` and `src/tools/local_tools.rs`  
   Align truncation response shape for tool consumers (marker vs explicit flag conventions).

9. `crates/sandbox-linux/src/native/seccomp.rs`  
   Document arch support expectations near seccomp profile constraints for non-x86_64 futures.

10. `crates/sandbox-linux/src/native/pid1.rs`  
    Consider defense-in-depth seccomp on PID1 after setup.

11. `crates/sandbox-windows/src/native/process.rs`  
    Remove dead-code import keeper helper in favor of explicit import hygiene.

12. `crates/sandbox-linux/src/request.rs`, `crates/sandbox-windows/src/request.rs`  
    Document intentional serialization asymmetry between Linux and Windows request types.

## Residual Risk / Test Gaps

- Linux native namespace paths were reviewed statically on this host; runtime Linux validation remains dependent on Linux CI/jobs.
- Miri is validated in CI (`sandbox-linux --lib`) but was not run locally in this pass.
- No blockers were found, but warning items 2/3/11 are the highest practical impact for runtime UX/streaming behavior.
