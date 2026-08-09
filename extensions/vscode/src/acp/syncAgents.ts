import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { AGENT_PROFILES, getWorkspaceRoot, listAgentIds } from "../paths";
import { getBinaryPath } from "../settings";

export interface AcpAgentEntry {
  command: string;
  args: string[];
}

export function buildAcpAgents(
  binaryPath: string,
  agentIds?: string[],
): Record<string, AcpAgentEntry> {
  const ids = agentIds?.length
    ? agentIds
    : AGENT_PROFILES.map((p) => p.id);
  const agents: Record<string, AcpAgentEntry> = {};
  for (const id of ids) {
    const profile = AGENT_PROFILES.find((p) => p.id === id);
    if (!profile) {
      continue;
    }
    agents[id] = {
      command: binaryPath,
      args: [
        "acp",
        "--project-root",
        "${workspaceFolder}",
        "--agent",
        id,
        "--home",
        profile.home,
      ],
    };
  }
  return agents;
}

export async function syncAcpAgents(
  workspaceRoot?: string,
): Promise<{ path: string; agentCount: number }> {
  const root = workspaceRoot ?? getWorkspaceRoot();
  if (!root) {
    throw new Error("No workspace folder open");
  }

  const vscodeDir = path.join(root, ".vscode");
  const settingsPath = path.join(vscodeDir, "settings.json");
  fs.mkdirSync(vscodeDir, { recursive: true });

  let settings: Record<string, unknown> = {};
  if (fs.existsSync(settingsPath)) {
    const raw = fs.readFileSync(settingsPath, "utf8");
    // Strip JSONC-ish comments roughly; VS Code settings are often plain JSON.
    const stripped = raw.replace(/^\s*\/\/.*$/gm, "").replace(/\/\*[\s\S]*?\*\//g, "");
    try {
      settings = JSON.parse(stripped || "{}") as Record<string, unknown>;
    } catch {
      throw new Error(`Failed to parse ${settingsPath}; fix JSON before syncing ACP agents`);
    }
  }

  const discovered = listAgentIds(root);
  const agents = buildAcpAgents(
    getBinaryPath(),
    discovered.length ? discovered : undefined,
  );
  settings["acp.agents"] = agents;

  fs.writeFileSync(settingsPath, `${JSON.stringify(settings, null, 2)}\n`, "utf8");
  return { path: settingsPath, agentCount: Object.keys(agents).length };
}

export async function syncAcpAgentsCommand(): Promise<void> {
  try {
    const result = await syncAcpAgents();
    vscode.window.showInformationMessage(
      `Synced ${result.agentCount} ACP agent(s) → ${path.basename(path.dirname(result.path))}/settings.json`,
    );
  } catch (err) {
    vscode.window.showErrorMessage(String(err));
  }
}
