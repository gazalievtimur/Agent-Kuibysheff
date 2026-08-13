# Releases

Prebuilt binaries for **Windows x86_64** and **Linux x86_64** are published on
[GitHub Releases](https://github.com/gybson63/Agent-Kuibysheff/releases)
when a version tag is pushed.

| Platform | Status | Archive |
| --- | --- | --- |
| Windows x86_64 | Supported | `agent_Kuibysheff-vX.Y.Z-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | Supported | `agent_Kuibysheff-vX.Y.Z-x86_64-unknown-linux-gnu.zip` |
| macOS (any) | **Unsupported** | Build-from-source at your own risk; no CI matrix |
| Linux aarch64 | **Unsupported** | Linux seccomp BPF is x86_64-only |

Linux release binaries are produced on GitHub Actions `ubuntu-latest`
(currently **Ubuntu 24.04**). Treat that runner’s **glibc** as the baseline;
older distributions may need a newer glibc or a local from-source build.

Each archive contains the `agent_Kuibysheff` binary. A matching
`.zip.sha256` checksum file is attached to the release.

The VS Code extension under `extensions/vscode` is versioned **independently**
of the CLI (extension may stay at `0.1.0` while the CLI is `0.2.x`).

PowerShell (Windows):

```powershell
# After downloading and extracting the zip:
.\agent_Kuibysheff-v0.2.0-x86_64-pc-windows-msvc.exe --help
```

Bash (Linux):

```bash
# After downloading and extracting the zip:
chmod +x ./agent_Kuibysheff-v0.2.0-x86_64-unknown-linux-gnu
./agent_Kuibysheff-v0.2.0-x86_64-unknown-linux-gnu --help
```

To cut a release from a commit on `main`:

```bash
# Keep Cargo.toml version in sync with the tag when bumping.
git tag v0.2.0
git push origin v0.2.0
```

The [Release](../.github/workflows/release.yml) workflow builds
`--release --locked --bin agent_Kuibysheff` on `windows-latest` and
`ubuntu-latest`, then uploads the zips to a GitHub Release for that tag.

See [CHANGELOG.md](../CHANGELOG.md) for migration notes (including the 0.2.0
required `access` policy). Install/upgrade/uninstall:
[INSTALL.md](INSTALL.md).

## Updating pinned GitHub Actions

CI pins third-party Actions to full commit SHAs. When Dependabot opens an
update PR, verify the new SHA matches the upstream tag, then merge. Prefer
Dependabot over hand-editing pins.
