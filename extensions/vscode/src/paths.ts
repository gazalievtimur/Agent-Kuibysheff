import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

export const AGENT_PROFILES = [
  { id: "1c-intake", stage: "1", home: "runs/vscode-active/stage1/home" },
  { id: "1c-analyst", stage: "2", home: "runs/vscode-active/stage2/home" },
  { id: "1c-coder", stage: "3", home: "runs/vscode-active/stage3/home" },
  { id: "1c-implementer", stage: "4", home: "runs/vscode-active/stage4/home" },
] as const;

export type AgentProfileId = (typeof AGENT_PROFILES)[number]["id"];

export function getWorkspaceRoot(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

export function kuibysheffRoot(workspaceRoot: string): string {
  return path.join(workspaceRoot, ".kuibysheff");
}

export function agentsRoot(workspaceRoot: string): string {
  return path.join(kuibysheffRoot(workspaceRoot), "protected", "agents");
}

export function agentDir(workspaceRoot: string, agentId: string): string {
  return path.join(agentsRoot(workspaceRoot), agentId);
}

export function agentConfigPath(workspaceRoot: string, agentId: string): string {
  return path.join(agentDir(workspaceRoot, agentId), "agent-config.yaml");
}

export function agentSettingsDir(workspaceRoot: string, agentId: string): string {
  return agentDir(workspaceRoot, agentId);
}

export function runsRoot(workspaceRoot: string): string {
  return path.join(kuibysheffRoot(workspaceRoot), "runs", "vscode-active");
}

export function artifactsRoot(workspaceRoot: string): string {
  return path.join(runsRoot(workspaceRoot), "artifacts");
}

export function stageHome(workspaceRoot: string, stage: string): string {
  return path.join(runsRoot(workspaceRoot), `stage${stage}`, "home");
}

export function listAgentIds(workspaceRoot: string): string[] {
  const root = agentsRoot(workspaceRoot);
  if (!fs.existsSync(root)) {
    return [];
  }
  return fs
    .readdirSync(root, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort();
}

export function hasKuibysheff(workspaceRoot: string): boolean {
  return fs.existsSync(kuibysheffRoot(workspaceRoot));
}

export function profileForAgent(agentId: string) {
  return AGENT_PROFILES.find((p) => p.id === agentId);
}

export function chatStarterPath(workspaceRoot: string, stage: string): string {
  return path.join(stageHome(workspaceRoot, stage), "CHAT_STARTER.txt");
}

export function stagePromptPath(workspaceRoot: string, stage: string): string {
  return path.join(stageHome(workspaceRoot, stage), "stage_prompt.md");
}

export function artifactFlags(workspaceRoot: string): {
  brief: boolean;
  plan: boolean;
  approved: boolean;
  code: boolean;
  cfe: boolean;
} {
  const art = artifactsRoot(workspaceRoot);
  return {
    brief: fs.existsSync(path.join(art, "brief")),
    plan: fs.existsSync(path.join(art, "plan")),
    approved: fs.existsSync(path.join(art, "plan", "APPROVED")),
    code: fs.existsSync(path.join(art, "code")),
    cfe: fs.existsSync(path.join(art, "cfe")),
  };
}
