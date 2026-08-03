import * as vscode from "vscode";

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

export async function ensureRepoRoot(): Promise<string | undefined> {
  let root = getRepoRoot();
  if (root) {
    return root;
  }

  const guessed = await guessRepoRoot();
  if (guessed) {
    const pick = await vscode.window.showInformationMessage(
      `Use detected Kuibyshev install?\n${guessed}`,
      "Use",
      "Browse…",
    );
    if (pick === "Use") {
      await vscode.workspace
        .getConfiguration("kuibyshev")
        .update("repoRoot", guessed, vscode.ConfigurationTarget.Workspace);
      return guessed;
    }
    if (pick !== "Browse…") {
      return undefined;
    }
  }

  const uris = await vscode.window.showOpenDialog({
    canSelectFiles: false,
    canSelectFolders: true,
    canSelectMany: false,
    openLabel: "Select Kuibyshev install",
  });
  if (!uris?.[0]) {
    return undefined;
  }
  root = uris[0].fsPath;
  await vscode.workspace
    .getConfiguration("kuibyshev")
    .update("repoRoot", root, vscode.ConfigurationTarget.Workspace);
  return root;
}

async function guessRepoRoot(): Promise<string | undefined> {
  const folders = vscode.workspace.workspaceFolders ?? [];
  for (const folder of folders) {
    const marker = vscode.Uri.joinPath(folder.uri, "scripts", "1c-dev-scaffold-project.ps1");
    try {
      await vscode.workspace.fs.stat(marker);
      return folder.uri.fsPath;
    } catch {
      // not this folder
    }
  }
  return undefined;
}
