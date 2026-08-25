# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking

- The CLI executable is renamed from `agent_Kuibysheff` to **`kbshff`**. The
  Cargo package/crate name remains `agent_Kuibysheff`. Update `PATH`, scripts,
  and `kuibysheff.binaryPath` accordingly. Release archives still use the
  `agent_Kuibysheff-v…` zip name; the binary inside is `kbshff`.

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

## [0.2.0] - 2026-08-09

### Breaking

- Agent config now **requires** an `access` policy. Omitting `access` fails
  closed at load time.
- Permissive home/workspace semantics are available only via an explicit
  opt-in: `access: { mode: legacy }`. Prefer fail-closed tools, path grants,
  and `home.run` program aliases in production. Details:
  [CONTRACT.md](CONTRACT.md#access-policy-fail-closed).

### Migration

1. Add an `access` block to each agent config (see
   [`agent-config.example.yaml`](agent-config.example.yaml)).
2. If you temporarily need pre-0.2 permissive FS behavior, set
   `access: { mode: legacy }` and plan a move to strict grants.
3. Re-run `kbshff check --project-root <DIR> --agent <ID>`.

### Notes

- CLI / crate version: **0.2.0**.
- VS Code extension (`extensions/vscode`) is versioned independently and may
  remain at **0.1.0** while the CLI is 0.2.x.

## [0.1.0] - 2026-07-01

Initial tagged release line (Windows/Linux x86_64 binaries).
