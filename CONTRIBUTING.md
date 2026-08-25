# Contributing

Thanks for contributing to **agent_Kuibysheff**.

## License

By submitting a contribution (pull request, patch, or other intentional
submission for inclusion), you agree that your contribution is licensed
under the **Apache License, Version 2.0**, without additional terms, as
described in section 5 of that license. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).

Please include a `Signed-off-by` line in each commit (Developer Certificate
of Origin):

```text
Signed-off-by: Your Name <you@example.com>
```

Do not copy code under licenses incompatible with Apache-2.0 (for example
GPL-2.0-only without an exception that permits combination).

## Trademarks

The Apache License does not grant rights to use the names **Kuibysheff**,
**agent_Kuibysheff**, or related marks except for reasonable attribution.

## Development setup

| Tool | Requirement |
| --- | --- |
| Rust | **1.88+** (MSRV; see `rust-version` in `Cargo.toml`) |
| Node.js | **20+** (VS Code extension under `extensions/vscode`) |
| Python | **3.12+** (coverage ratchet / local eval scripts) |
| Docker / WSL | Optional; required for some Linux sandbox / SWE-bench / security lab flows |

Clone with submodules (Cursor rust-skills under `.cursor/skills/rust-skills`):

```bash
git clone --recurse-submodules https://github.com/gazalievtimur/Agent-Kuibysheff.git
cd Agent-Kuibysheff
# or, if already cloned:
git submodule update --init --recursive
```

Install local git hooks:

```powershell
.\scripts\install-git-hooks.ps1
```

```bash
./scripts/install-git-hooks.sh
```

## Checks

Run the offline quality gate before opening a PR:

```powershell
.\scripts\check.ps1
```

```bash
./scripts/check.sh
```

Both run `fmt`, Clippy, `cargo deny`, tests, and portability checks. When a local
`workflows/` tree is present (gitignored copy-units), they also run detached
workflow smoke tests. Live LLM regressions are **opt-in**:

- AoC: `-Aoc` / `--aoc` or `RUN_AOC=1` → delegates to [kuibysheff-aoc](https://github.com/gazalievtimur/kuibysheff-aoc) (`KUIBYSHEFF_AOC_ROOT` or sibling clone)
- SWE-bench: `-Swebench` / `--swebench` or `RUN_SWEBENCH=1` → [kuibysheff-swebench](https://github.com/gazalievtimur/kuibysheff-swebench)
- Security sandbox: `-Security` / `--security` or `RUN_SECURITY=1` (local `workflows/`)
- Scale-FS: `-ScaleFs` / `--scale-fs` or `RUN_SCALE_FS=1` (local `workflows/`)

1C CF/CFE live eval is a separate example repo:
[kuibysheff-1c-live](https://github.com/gazalievtimur/kuibysheff-1c-live) (not hooked to `check`).

Install `cargo-deny` if needed: `cargo install --locked cargo-deny`.

## Pull requests

- Keep changes focused; include tests for behavior changes when practical.
- Update docs (`README`, `CONTRACT.md`, `docs/`) when user-visible behavior
  changes.
- Call out **security impact** in the PR template (sandbox, MCP, secrets,
  access policy).
- Do not commit secrets, `.env`, local banks, or machine-specific paths.
- Follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Documentation language

User-facing and governance docs (README, SECURITY, CONTRIBUTING, SUPPORT,
templates, CHANGELOG) are maintained in **English**. Russian engineering notes
(for example under `docs/architecture-review/`) may remain bilingual or
Russian; they are engineering backlog, not product documentation.

## Security

See [SECURITY.md](SECURITY.md). Do not disclose exploitable details in public
issues.
