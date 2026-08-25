# Install, upgrade, and uninstall

Supported platforms for prebuilt binaries: **Windows x86_64** and **Linux
x86_64**. macOS and Linux aarch64 are **unsupported** (see
[RELEASING.md](docs/RELEASING.md)).

## Install (prebuilt)

1. Download the archive and `.zip.sha256` for your platform from
   [GitHub Releases](https://github.com/gazalievtimur/Agent-Kuibysheff/releases).
2. Verify the checksum (example):

```powershell
# Windows PowerShell
Get-FileHash .\agent_Kuibysheff-v0.2.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Get-Content .\agent_Kuibysheff-v0.2.0-x86_64-pc-windows-msvc.zip.sha256
```

```bash
# Linux
sha256sum -c agent_Kuibysheff-v0.2.0-x86_64-unknown-linux-gnu.zip.sha256
```

3. Extract the binary and place it on your `PATH`.
4. Confirm: `kbshff --help`.

Linux binaries are built on GitHub `ubuntu-latest` (currently Ubuntu 24.04)
against that runner’s **glibc**; older distros may need a newer glibc or a
from-source build.

## Install (from source)

Requirements: **Rust 1.88+** (MSRV), plus platform toolchains for your OS.

```bash
git clone --recurse-submodules https://github.com/gazalievtimur/Agent-Kuibysheff.git
cd Agent-Kuibysheff
cargo build --release --bin kbshff
```

The CLI binary is named **`kbshff`** (crate/package remains `agent_Kuibysheff`).
Running `kbshff` with no arguments opens an interactive setup wizard (TTY
required). The wizard can store the provider API key in the agent profile
`.env` so you do not need to export it in every shell session.

## Upgrade

- **Prebuilt:** replace the binary with the newer release archive; re-check
  checksums.
- **From source:** `git pull` (update submodules if needed) and rebuild.
- Read [CHANGELOG.md](CHANGELOG.md) for breaking config changes (for example
  required `access` in 0.2.0).

User data under each project’s `.kuibysheff/` (protected profiles, homes, runs)
is **not** overwritten by installing a new binary.

## Uninstall

1. Remove the `kbshff` binary from your install location / `PATH`.
2. Optionally delete project data:
   - `.kuibysheff/protected/` — agent profiles and protected store;
   - `.kuibysheff/homes/`, `.kuibysheff/runs/` — run workspaces and artifacts;
   - local `.env` / `agent-config.local.yaml` if you created them.
3. VS Code extension: uninstall from the Extensions view, or remove a locally
   installed `.vsix`. Extension versioning is independent of the CLI.

Uninstall does **not** automatically delete `.kuibysheff/` trees; remove them
only if you no longer need agent configuration or run history.
