# Kuibyshev VS Code extension

Manage Kuibyshev agent parameters and the 1C ACP workflow from the sidebar.

This extension does **not** implement the chat UI (AHP). VS Code’s Agent Host still talks ACP to `agent_Kuibyshev acp`. The extension configures YAML, syncs `acp.agents`, scaffolds `.kuibyshev/`, and runs prepare / promote / validate / approve via the existing PowerShell scripts.

## Features

- **Sidebar** — agents under `.kuibyshev/agents`, workflow stage status, quick actions
- **Config editor** — provider, limits, MCP, access, logging (webview); Validate runs `agent_Kuibyshev check`
- **Sync ACP** — writes `acp.agents` into `.vscode/settings.json`
- **Scaffold** — wraps `scripts/1c-dev-scaffold-project.ps1`
- **Workflow** — wraps `scripts/1c-dev-acp-prepare.ps1` (prepare / promote / validate / approve)

## Requirements

- Windows + PowerShell (same as current 1C workflow scripts)
- `agent_Kuibyshev` on `PATH` (or set `kuibyshev.binaryPath`)
- Kuibyshev install path in `kuibyshev.repoRoot` (scripts + templates)

## Settings

| Setting | Description |
|---------|-------------|
| `kuibyshev.repoRoot` | Absolute path to the Agent Kuibyshev clone |
| `kuibyshev.binaryPath` | Binary name/path (default `agent_Kuibyshev`) |
| `kuibyshev.defaultIssueKey` | Default issue key for Prepare |

## Develop / run

From the repository root:

```powershell
cd extensions/vscode
npm install
npm run compile
```

Then in VS Code: **Run and Debug → Run Kuibyshev Extension** (F5). That launches an Extension Development Host with this package.

Or install a local VSIX after `npx @vscode/vsce package` from `extensions/vscode`.

## Typical flow

1. Open the **product** folder in VS Code (not necessarily this repo).
2. Set `kuibyshev.repoRoot` to your Kuibyshev install.
3. Command **Kuibyshev: Scaffold project** (or sidebar Actions).
4. Open an agent → edit provider / MCP / `workspace.root` → **Save** → **Validate**.
5. **Prepare stage** → copy chat starter → chat with the matching ACP agent (`1c-analyst`, …).
6. Promote / Validate / Approve plan as needed.

See also [`workflows/1c-dev/VSCODE.md`](../../workflows/1c-dev/VSCODE.md).
