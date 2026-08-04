import * as vscode from "vscode";
import { registerCommands } from "./commands";
import { AgentsTreeProvider } from "./views/agentsTree";

export function activate(context: vscode.ExtensionContext): void {
  const tree = new AgentsTreeProvider();
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("kuibyshev.agents", tree),
  );

  registerCommands(context, tree);

  const watcher = vscode.workspace.createFileSystemWatcher("**/.kuibyshev/**");
  watcher.onDidCreate(() => tree.refresh());
  watcher.onDidChange(() => tree.refresh());
  watcher.onDidDelete(() => tree.refresh());
  context.subscriptions.push(watcher);
}

export function deactivate(): void {
  // no-op
}
