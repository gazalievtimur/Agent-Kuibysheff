import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  AGENT_PROFILES,
  artifactFlags,
  getWorkspaceRoot,
  hasKuibyshev,
  listAgentIds,
  stageHome,
} from "../paths";

export type TreeNodeKind = "section" | "agent" | "action" | "info";

export class KuibyshevTreeItem extends vscode.TreeItem {
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
        command: "kuibyshev.editAgent",
        title: "Edit agent",
        arguments: [{ agentId }],
      };
      const configPath = path.join(
        getWorkspaceRoot() ?? "",
        ".kuibyshev",
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

export class AgentsTreeProvider implements vscode.TreeDataProvider<KuibyshevTreeItem> {
  private readonly _onDidChangeTreeData = new vscode.EventEmitter<
    KuibyshevTreeItem | undefined | void
  >();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getTreeItem(element: KuibyshevTreeItem): vscode.TreeItem {
    return element;
  }

  getChildren(element?: KuibyshevTreeItem): KuibyshevTreeItem[] {
    const root = getWorkspaceRoot();
    if (!element) {
      if (!root) {
        return [
          new KuibyshevTreeItem("Open a product folder workspace", "info"),
        ];
      }
      if (!hasKuibyshev(root)) {
        return [
          new KuibyshevTreeItem("No .kuibyshev — run Scaffold", "info"),
          new KuibyshevTreeItem("Scaffold project", "action", undefined, "kuibyshev.scaffold"),
        ];
      }
      return [
        new KuibyshevTreeItem("Agents", "section"),
        new KuibyshevTreeItem("Workflow", "section"),
        new KuibyshevTreeItem("Actions", "section"),
      ];
    }

    if (!root) {
      return [];
    }

    if (element.label === "Agents") {
      const ids = listAgentIds(root);
      if (!ids.length) {
        return [new KuibyshevTreeItem("No agent profiles", "info")];
      }
      return ids.map((id) => new KuibyshevTreeItem(id, "agent", id));
    }

    if (element.label === "Workflow") {
      const flags = artifactFlags(root);
      const items: KuibyshevTreeItem[] = [];
      for (const profile of AGENT_PROFILES) {
        const home = stageHome(root, profile.stage);
        const ready = fs.existsSync(path.join(home, "stage_prompt.md"));
        items.push(
          new KuibyshevTreeItem(
            `Stage ${profile.stage} (${profile.id}): ${ready ? "prepared" : "idle"}`,
            "info",
          ),
        );
      }
      items.push(
        new KuibyshevTreeItem(
          `Artifacts: brief=${yn(flags.brief)} plan=${yn(flags.plan)} approved=${yn(flags.approved)} code=${yn(flags.code)} cfe=${yn(flags.cfe)}`,
          "info",
        ),
      );
      return items;
    }

    if (element.label === "Actions") {
      return [
        new KuibyshevTreeItem("Prepare stage", "action", undefined, "kuibyshev.prepare"),
        new KuibyshevTreeItem("Promote", "action", undefined, "kuibyshev.promote"),
        new KuibyshevTreeItem("Validate", "action", undefined, "kuibyshev.validate"),
        new KuibyshevTreeItem(
          "Promote + Validate",
          "action",
          undefined,
          "kuibyshev.promoteAndValidate",
        ),
        new KuibyshevTreeItem("Approve plan", "action", undefined, "kuibyshev.approvePlan"),
        new KuibyshevTreeItem("Copy chat starter", "action", undefined, "kuibyshev.copyChatStarter"),
        new KuibyshevTreeItem("Sync ACP agents", "action", undefined, "kuibyshev.syncAcp"),
        new KuibyshevTreeItem("Scaffold project", "action", undefined, "kuibyshev.scaffold"),
      ];
    }

    return [];
  }
}

function yn(v: boolean): string {
  return v ? "yes" : "no";
}
