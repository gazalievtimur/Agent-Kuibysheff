# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
3. Re-run `agent_Kuibysheff check --project-root <DIR> --agent <ID>`.

### Notes

- CLI / crate version: **0.2.0**.
- VS Code extension (`extensions/vscode`) is versioned independently and may
  remain at **0.1.0** while the CLI is 0.2.x.

## [0.1.0] - 2026-07-01

Initial tagged release line (Windows/Linux x86_64 binaries).
