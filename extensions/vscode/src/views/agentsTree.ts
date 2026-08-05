import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  AGENT_PROFILES,
  artifactFlags,
  getWorkspaceRoot,
  hasKuibysheff,
  listAgentIds,
  stageHome,
} from "../paths";

export type TreeNodeKind = "section" | "agent" | "action" | "info";

export class KuibysheffTreeItem extends vscode.TreeItem {
  constructor(
    public readonly label: string,
    public readonly kind: TreeNodeKind,
    public readonly agentId?: string,
    public readonly commandId?: string,
  ) {
    super(
      label,
      kind === "section"
        ? vscode.TreeItemCollapsibleState.Expanded
        : vscode.TreeItemCollapsibleState.None,
    );
    this.contextValue = kind;
    if (kind === "agent" && agentId) {
      this.iconPath = new vscode.ThemeIcon("robot");
      this.command = {
        command: "kuibysheff.editAgent",
        title: "Edit agent",
        arguments: [{ agentId }],
      };
      const configPath = path.join(
        getWorkspaceRoot() ?? "",
        ".kuibysheff",
        "agents",
        agentId,
        "agent-config.yaml",
      );
      this.description = fs.existsSync(configPath) ? "config" : "missing config";
      this.tooltip = configPath;
    }
    if (kind === "action" && commandId) {
      this.iconPath = new vscode.ThemeIcon("play");
      this.command = { command: commandId, title: label };
    }
    if (kind === "info") {
      this.iconPath = new vscode.ThemeIcon("info");
    }
  }
}

export class AgentsTreeProvider implements vscode.TreeDataProvider<KuibysheffTreeItem> {
  private readonly _onDidChangeTreeData = new vscode.EventEmitter<
    KuibysheffTreeItem | undefined | void
  >();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getTreeItem(element: KuibysheffTreeItem): vscode.TreeItem {
    return element;
  }

  getChildren(element?: KuibysheffTreeItem): KuibysheffTreeItem[] {
    const root = getWorkspaceRoot();
    if (!element) {
      if (!root) {
        return [
          new KuibysheffTreeItem("Open a product folder workspace", "info"),
        ];
      }
      if (!hasKuibysheff(root)) {
        return [
          new KuibysheffTreeItem("No .kuibysheff — run Scaffold", "info"),
          new KuibysheffTreeItem("Scaffold project", "action", undefined, "kuibysheff.scaffold"),
        ];
      }
      return [
        new KuibysheffTreeItem("Agents", "section"),
        new KuibysheffTreeItem("Workflow", "section"),
        new KuibysheffTreeItem("Actions", "section"),
      ];
    }

    if (!root) {
      return [];
    }

    if (element.label === "Agents") {
      const ids = listAgentIds(root);
      if (!ids.length) {
        return [new KuibysheffTreeItem("No agent profiles", "info")];
      }
      return ids.map((id) => new KuibysheffTreeItem(id, "agent", id));
    }

    if (element.label === "Workflow") {
      const flags = artifactFlags(root);
      const items: KuibysheffTreeItem[] = [];
      for (const profile of AGENT_PROFILES) {
        const home = stageHome(root, profile.stage);
        const ready = fs.existsSync(path.join(home, "stage_prompt.md"));
        items.push(
          new KuibysheffTreeItem(
            `Stage ${profile.stage} (${profile.id}): ${ready ? "prepared" : "idle"}`,
            "info",
          ),
        );
      }
      items.push(
        new KuibysheffTreeItem(
          `Artifacts: brief=${yn(flags.brief)} plan=${yn(flags.plan)} approved=${yn(flags.approved)} code=${yn(flags.code)} cfe=${yn(flags.cfe)}`,
          "info",
        ),
      );
      return items;
    }

    if (element.label === "Actions") {
      return [
        new KuibysheffTreeItem("Prepare stage", "action", undefined, "kuibysheff.prepare"),
        new KuibysheffTreeItem("Promote", "action", undefined, "kuibysheff.promote"),
        new KuibysheffTreeItem("Validate", "action", undefined, "kuibysheff.validate"),
        new KuibysheffTreeItem(
          "Promote + Validate",
          "action",
          undefined,
          "kuibysheff.promoteAndValidate",
        ),
        new KuibysheffTreeItem("Approve plan", "action", undefined, "kuibysheff.approvePlan"),
        new KuibysheffTreeItem("Copy chat starter", "action", undefined, "kuibysheff.copyChatStarter"),
        new KuibysheffTreeItem("Sync ACP agents", "action", undefined, "kuibysheff.syncAcp"),
        new KuibysheffTreeItem("Scaffold project", "action", undefined, "kuibysheff.scaffold"),
      ];
    }

    return [];
  }
}

function yn(v: boolean): string {
  return v ? "yes" : "no";
}
