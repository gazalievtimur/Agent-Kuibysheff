# Kuibysheff VS Code extension

Manage Kuibysheff agent parameters and the 1C ACP workflow from the sidebar.

Extension package version (`0.1.0` in `package.json`) is **independent** of the
CLI crate version (`0.2.x`).

This extension does **not** implement the chat UI (AHP). VS Code’s Agent Host still talks ACP to `kbshff acp`. The extension configures YAML, syncs `acp.agents`, scaffolds `.kuibysheff/`, and runs prepare / promote / validate / approve via the existing PowerShell scripts.

## Features

- **Sidebar** — agents under `.kuibysheff/protected/agents`, workflow stage status, quick actions
- **Config editor** — provider, limits, MCP, access, logging (webview); Validate runs `kbshff check`
- **Sync ACP** — writes `acp.agents` into `.vscode/settings.json`
- **Scaffold** — wraps `scripts/1c-dev-scaffold-project.ps1`
- **Workflow** — wraps `scripts/1c-dev-acp-prepare.ps1` (prepare / promote / validate / approve)

## Requirements

- Windows + PowerShell (same as current 1C workflow scripts)
- `kbshff` on `PATH` (or set `kuibysheff.binaryPath`)
- Kuibysheff install path in `kuibysheff.repoRoot` (scripts + templates)

## Settings

| Setting | Description |
|---------|-------------|
| `kuibysheff.repoRoot` | Absolute path to the **Agent Kuibysheff** clone (with `scripts/…`), **not** the product folder |
| `kuibysheff.binaryPath` | Binary name/path (default `kbshff`) |
| `kuibysheff.defaultIssueKey` | Default issue key for Prepare |

## Develop / run

From the repository root:

```powershell
cd extensions/vscode
npm install
npm run compile
```

Then in VS Code: **Run and Debug → Run Kuibysheff Extension** (F5). That launches an Extension Development Host with this package.

Or install a local VSIX after `npx @vscode/vsce package` from `extensions/vscode`.

## Typical flow

1. Open the **product** folder in VS Code (not necessarily this repo).
2. Set `kuibysheff.repoRoot` to your Kuibysheff install.
3. Command **Kuibysheff: Scaffold project** (or sidebar Actions).
4. Open an agent → edit provider / MCP / `workspace.root` → **Save** → **Validate**.
5. **Prepare stage** → copy chat starter → chat with the matching ACP agent (`1c-analyst`, …).
6. Promote / Validate / Approve plan as needed.

See also monorepo scripts `scripts/1c-dev-scaffold-project.ps1` /
`scripts/1c-dev-acp-prepare.ps1`. The full 1C copy-unit docs live under local
`workflows/1c-dev/` when restored from git history.
