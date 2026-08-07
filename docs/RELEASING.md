# Releases

Prebuilt binaries for Windows and Linux are published on
[GitHub Releases](https://github.com/gybson63/Agent-Kuibysheff/releases)
when a version tag is pushed.

| Platform | Archive |
| --- | --- |
| Windows x86_64 | `agent_Kuibysheff-vX.Y.Z-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `agent_Kuibysheff-vX.Y.Z-x86_64-unknown-linux-gnu.zip` |

Each archive contains the `agent_Kuibysheff` binary. A matching
`.zip.sha256` checksum file is attached to the release.

PowerShell (Windows):

```powershell
# After downloading and extracting the zip:
.\agent_Kuibysheff-v0.1.0-x86_64-pc-windows-msvc.exe --help
```

Bash (Linux):

```bash
# After downloading and extracting the zip:
chmod +x ./agent_Kuibysheff-v0.1.0-x86_64-unknown-linux-gnu
./agent_Kuibysheff-v0.1.0-x86_64-unknown-linux-gnu --help
```

To cut a release from a commit on `main`:

```bash
# Keep Cargo.toml version in sync with the tag when bumping.
git tag v0.1.0
git push origin v0.1.0
```

The [Release](../.github/workflows/release.yml) workflow builds
`--release --locked --bin agent_Kuibysheff` on `windows-latest` and
`ubuntu-latest`, then uploads the zips to a GitHub Release for that tag.
