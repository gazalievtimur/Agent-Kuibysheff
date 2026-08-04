import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { getWorkspaceRoot } from "./paths";

const SCAFFOLD_SCRIPT = path.join("scripts", "1c-dev-scaffold-project.ps1");

export function getRepoRoot(): string {
  return vscode.workspace.getConfiguration("kuibyshev").get<string>("repoRoot", "").trim();
}

export function getBinaryPath(): string {
  const value = vscode.workspace
    .getConfiguration("kuibyshev")
    .get<string>("binaryPath", "agent_Kuibyshev")
    .trim();
  return value || "agent_Kuibyshev";
}

export function getDefaultIssueKey(): string {
  return vscode.workspace
    .getConfiguration("kuibyshev")
    .get<string>("defaultIssueKey", "")
    .trim();
}

/** True when `root` looks like an Agent Kuibyshev install (has scaffold script). */
export function isKuibyshevInstall(root: string): boolean {
  if (!root.trim()) {
    return false;
  }
  return fs.existsSync(path.join(root, SCAFFOLD_SCRIPT));
}

/**
 * Resolve the Agent Kuibyshev install directory (scripts + templates).
 * This is NOT the product/workspace folder opened in VS Code.
 */
export async function ensureRepoRoot(): Promise<string | undefined> {
  const configured = getRepoRoot();
  if (configured && isKuibyshevInstall(configured)) {
    return configured;
  }

  if (configured && !isKuibyshevInstall(configured)) {
    const workspace = getWorkspaceRoot();
    const looksLikeProduct =
      workspace !== undefined &&
      path.resolve(configured).toLowerCase() === path.resolve(workspace).toLowerCase();

    const detail = looksLikeProduct
      ? "kuibyshev.repoRoot is set to this product folder. It must point to the Agent Kuibyshev install (the repo with scripts/1c-dev-scaffold-project.ps1), e.g. C:\\Git\\Agent Kuibyshev."
      : `kuibyshev.repoRoot is invalid (scaffold script missing):\n${configured}\n\nSelect the Agent Kuibyshev install folder.`;

    const pick = await vscode.window.showWarningMessage(
      detail,
      "Browse…",
      "Cancel",
    );
    if (pick !== "Browse…") {
      return undefined;
    }
    return browseAndSaveRepoRoot();
  }

  const guessed = await guessRepoRoot();
  if (guessed) {
    const pick = await vscode.window.showInformationMessage(
      `Use detected Kuibyshev install?\n${guessed}`,
      "Use",
      "Browse…",
    );
    if (pick === "Use") {
      await saveRepoRoot(guessed);
      return guessed;
    }
    if (pick !== "Browse…") {
      return undefined;
    }
    return browseAndSaveRepoRoot();
  }

  const pick = await vscode.window.showInformationMessage(
    "Set kuibyshev.repoRoot to your Agent Kuibyshev install (not the product folder).",
    "Browse…",
  );
  if (pick !== "Browse…") {
    return undefined;
  }
  return browseAndSaveRepoRoot();
}

async function browseAndSaveRepoRoot(): Promise<string | undefined> {
  const uris = await vscode.window.showOpenDialog({
    canSelectFiles: false,
    canSelectFolders: true,
    canSelectMany: false,
    openLabel: "Select Agent Kuibyshev install",
    title: "Agent Kuibyshev install (contains scripts/1c-dev-scaffold-project.ps1)",
  });
  if (!uris?.[0]) {
    return undefined;
  }
  const root = uris[0].fsPath;
  if (!isKuibyshevInstall(root)) {
    vscode.window.showErrorMessage(
      `Not a Kuibyshev install (missing ${SCAFFOLD_SCRIPT}):\n${root}`,
    );
    return undefined;
  }
  await saveRepoRoot(root);
  return root;
}

async function saveRepoRoot(root: string): Promise<void> {
  await vscode.workspace
    .getConfiguration("kuibyshev")
    .update("repoRoot", root, vscode.ConfigurationTarget.Workspace);
}

async function guessRepoRoot(): Promise<string | undefined> {
  const folders = vscode.workspace.workspaceFolders ?? [];
  for (const folder of folders) {
    if (isKuibyshevInstall(folder.uri.fsPath)) {
      return folder.uri.fsPath;
    }
  }
  return undefined;
}
