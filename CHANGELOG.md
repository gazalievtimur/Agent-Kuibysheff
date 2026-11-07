# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Migrate `agent-client-protocol` from 3.x to 5.x (`Agent.v1()` / `Client.v1()`,
  `v1::schema`). The wire protocol stays ACP schema v1.

## [0.3.1] - 2026-11-04

### Added

- Resume interrupted worker runs from a saved chat transcript:
  `kbshff run --continue-from <chat_history.json>`. The engine seeds the model
  loop from that file, replaces the leading system message with the current
  profile prompt, and optionally appends a new user turn when `--prompt` is
  also set. Limits apply only to the continuation.
- Studio Test run **Продолжить** button next to **Start run**. It posts
  `continue_from_chat: true` on `POST /api/agents/:id/runs`, which spawns
  `kbshff run --continue-from {home}/logs/chat_history.json` and always
  enables `--save-chat-history`. A missing transcript returns HTTP 400.

### Changed

- Document resume semantics in `CONTRACT.md`, `docs/UI.md`, and
  `docs/LOGGING.md` (chat history is the full unpruned transcript used for
  resume, not the pruned model window).

## [0.3.0] - 2026-11-03

### Added

- Browser operator studio (`kbshff-studio` / `kbshff ui`): local HTTP console
  that drives `kbshff` as a subprocess for profile edit, check, MCP probe, and
  a single live run with SSE progress and cancel.
- Raw `home.write`, pinned context, progress/cancel events, and operator CLI
  paths that back the studio without changing the worker contract.
- Skill instruction packages: `skill.md` plus companion files. The system
  prompt keeps a catalog (`policy`, `instructions=stored|none`, file count).
  The model reads the folder with `profile.read_skill` and `profile.list_skill`.
  Manage the files with `config skill body`, `config skill file`, directory
  import, or the studio Configure tab (`skill.md` only).
- Skill packages for a Kuibysheff profile load from
  `.kuibysheff/runtime/skills/{agent}/` so sandboxed `home.run` can read
  companion scripts without a `protected/` grant. Missing runtime falls back
  to `protected/agents/{agent}/skills`; when both exist, runtime wins.

### Changed

- Migrate `agent-client-protocol` to 3.x with the `stdio` feature for
  `Stdio::new()`.
- Dependency bumps: `tokio` 1.54, `js-yaml` 5.5, `actions/setup-python` 7.1,
  `Swatinem/rust-cache` 2.9.3, and related pins.

### Fixed

- Bump `pulldown-cmark` to 0.13.6 for
  [RUSTSEC-2026-0414](https://rustsec.org/advisories/RUSTSEC-2026-0414).
- Bump `rustls` / `webpki` advisories that blocked Dependabot CI after 0.2.2.

## [0.2.2] - 2026-10-07

### Fixed

- Bump `rustls` to 0.23.45 for [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285).
- VS Code extension: binary fallback looks up `kbshff` after the CLI rename.
- Coverage ratchet: decode `git diff` output as UTF-8 on Windows.
- Apply rust-skills remediations for numeric casts, `spawn_blocking` joins, and
  tool-loop clones; add coverage for seccomp `bpf_prog_len` and home join mapping.
- A2A server loopback tests: bypass the system HTTP proxy via a typed client.

### Changed

- Split the MCP stdio client into process, registry, and executor modules.
- Dependency bumps: `rand` 0.10, `sha2` 0.11, `base64` 0.23, `clap`,
  `agent-client-protocol`, `thiserror`, `rust_decimal`, and related VS Code
  npm pins / GitHub Actions.

### Added

- Kuibyshev app icon assets and wiring in the VS Code extension.
- Architecture-review follow-ups from the 2026-09 rust-skills review.

## [0.2.1] - 2026-09-02

### Fixed

- Linux sandbox: fail closed when a timeout cannot reap the child (`waitid` /
  missing pidfd paths); bound waits so a D-state helper cannot hang CI.
- Windows sandbox: stop reclaim from deleting live AppContainer profiles.
- Serialize Linux namespace tests to one thread to avoid unshare deadlocks.
- Coverage ratchet: fetch `origin/<base>` so the changed-lines gate works on PRs.
- Nightly / release workflows: job timeouts so hangs fail in minutes.
- `run-demo` / demo config: grant `access.filesystem.input_roots` so attached
  project files (for example `README.md`) are allowed after protected import.
- Scale-FS probe example: raise `max_tokens` and tighten history so oversize
  windowed-read tasks are less likely to stop on the token budget.

### Changed

- Miri skips process-spawn sandbox unit tests that need real OS isolation.

## [0.2.0] - 2026-08-26

### Breaking

- The CLI executable is renamed from `agent_Kuibysheff` to **`kbshff`**. The
  Cargo package/crate name remains `agent_Kuibysheff`. Update `PATH`, scripts,
  and `kuibysheff.binaryPath` accordingly. Release archives still use the
  `agent_Kuibysheff-v…` zip name; the binary inside is `kbshff`.
- Agent config now **requires** an `access` policy. Omitting `access` fails
  closed at load time.
- Permissive home/workspace semantics are available only via an explicit
  opt-in: `access: { mode: legacy }`. Prefer fail-closed tools, path grants,
  and `home.run` program aliases in production. Details:
  [CONTRACT.md](CONTRACT.md#access-policy-fail-closed).

### Added

- Launching `kbshff` with **no subcommand** on a TTY opens an interactive setup
  wizard: choose harness folder (default: cwd), create or select an agent
  profile, run resource checks with a connected/available inventory, then
  optionally continue configuring (MCP, provider, limits).
- Agent ids may use Unicode letters/digits, spaces, `_`, and `-` (still a single
  path segment; path separators and reserved filename characters are rejected).
  The interactive wizard re-prompts on an invalid id instead of exiting.
- Interactive setup asks for the API key value once and writes it to the agent
  profile `.env` (alongside `api_key_env` in YAML). The key is not stored in
  `agent-config.yaml`. Dotenv is loaded before wizard `check` so the new key is
  visible immediately.
- **`kbshff a2a`**: Agent-to-Agent (A2A) Protocol 1.0 HTTP server via the
  official Linux Foundation Rust SDK (`a2a-lf` / `a2a-server-lf`). Exposes
  `/.well-known/agent-card.json`, JSON-RPC at `/jsonrpc`, and HTTP+JSON at
  `/rest`. Each `SendMessage` runs one worker turn (`run_agent_prompt`). Default
  bind is `127.0.0.1:8787`; optional `--token-env` requires Bearer on RPC/REST.

### Changed

- Product docs and script defaults use `OPENAI_API_KEY` / generic OpenAI-compatible
  URLs; Polza-specific branding was removed from examples.

### Migration

1. Add an `access` block to each agent config (see
   [`agent-config.example.yaml`](agent-config.example.yaml)).
2. If you temporarily need pre-0.2 permissive FS behavior, set
   `access: { mode: legacy }` and plan a move to strict grants.
3. Re-run `kbshff check --project-root <DIR> --agent <ID>`.
4. Rename the CLI binary on `PATH` from `agent_Kuibysheff` to `kbshff` (and update
   VS Code `kuibysheff.binaryPath` if used).

### Notes

- CLI / crate version: **0.2.0**.
- VS Code extension (`extensions/vscode`) is versioned independently and may
  remain at **0.1.0** while the CLI is 0.2.x.
- Security reports: [SECURITY.md](SECURITY.md) (`gazalievtimur@gmail.com`).

## [0.1.0] - 2026-07-01

Initial tagged release line (Windows/Linux x86_64 binaries).
