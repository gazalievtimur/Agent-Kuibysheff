import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { syncAcpAgents } from "../acp/syncAgents";
import {
  chatStarterPath,
  getWorkspaceRoot,
  listAgentIds,
  stagePromptPath,
} from "../paths";
import {
  prepareScript,
  runAgentCheck,
  runPowerShellScript,
  scaffoldScript,
} from "../process/runBinary";
import { ensureRepoRoot, getDefaultIssueKey, isKuibysheffInstall } from "../settings";
import { AgentsTreeProvider } from "../views/agentsTree";
import { openAgentConfigEditor } from "../views/configWebview";

const STAGE_OPTIONS = ["1", "2", "3", "4"];

let sharedOutput: vscode.OutputChannel | undefined;

function getOutput(): vscode.OutputChannel {
  if (!sharedOutput) {
    sharedOutput = vscode.window.createOutputChannel("Kuibysheff");
  }
  return sharedOutput;
}

function requireWorkspace(): string | undefined {
  const root = getWorkspaceRoot();
  if (!root) {
    vscode.window.showErrorMessage(
      "Open a product folder as the VS Code workspace first.",
    );
  }
  return root;
}

async function pickAgentId(preferred?: string): Promise<string | undefined> {
  const root = requireWorkspace();
  if (!root) {
    return undefined;
  }
  if (preferred) {
    return preferred;
  }
  const ids = listAgentIds(root);
  if (!ids.length) {
    vscode.window.showWarningMessage(
      "No agents under .kuibysheff/protected/agents. Run Kuibysheff: Scaffold project.",
    );
    return undefined;
  }
  return vscode.window.showQuickPick(ids, {
    placeHolder: "Select agent profile",
  });
}

async function pickStageQuick(title: string): Promise<string | undefined> {
  return vscode.window.showQuickPick(STAGE_OPTIONS, {
    title,
    placeHolder: "Stage 1–4",
  });
}

function agentIdFromArg(item: unknown): string | undefined {
  if (!item || typeof item !== "object") {
    return undefined;
  }
  if ("agentId" in item && typeof (item as { agentId: unknown }).agentId === "string") {
    return (item as { agentId: string }).agentId;
  }
  return undefined;
}

export function registerCommands(
  context: vscode.ExtensionContext,
  tree: AgentsTreeProvider,
): void {
  const register = (
    id: string,
    handler: (...args: unknown[]) => unknown,
  ): void => {
    context.subscriptions.push(vscode.commands.registerCommand(id, handler));
  };

  register("kuibysheff.refresh", () => tree.refresh());

  register("kuibysheff.scaffold", async () => {
    const workspaceRoot = requireWorkspace();
    if (!workspaceRoot) {
      return;
    }
    const repoRoot = await ensureRepoRoot();
    if (!repoRoot) {
      return;
    }
    if (!isKuibysheffInstall(repoRoot)) {
      vscode.window.showErrorMessage(
        `kuibysheff.repoRoot is not a Kuibysheff install: ${repoRoot}`,
      );
      return;
    }

    const force = await vscode.window.showWarningMessage(
      "Scaffold .kuibysheff into this workspace?",
      { modal: true },
      "Scaffold",
      "Force overwrite",
    );
    if (!force) {
      return;
    }

    const script = scaffoldScript(repoRoot);

    const args = ["-ProjectRoot", workspaceRoot, "-RepoRoot", repoRoot];
    if (force === "Force overwrite") {
      args.push("-Force");
    }

    const out = getOutput();
    out.show(true);
    out.appendLine(`> scaffold ${workspaceRoot}`);
    const result = await runPowerShellScript(script, args, workspaceRoot);
    out.appendLine(result.stdout);
    if (result.stderr) {
      out.appendLine(result.stderr);
    }

    if (result.code !== 0) {
      vscode.window.showErrorMessage(
        `Scaffold failed (exit ${result.code}). See Kuibysheff output.`,
      );
      return;
    }

    try {
      await syncAcpAgents(workspaceRoot);
    } catch (err) {
      vscode.window.showWarningMessage(
        `Scaffold OK, ACP sync failed: ${String(err)}`,
      );
    }

    tree.refresh();
    const first = listAgentIds(workspaceRoot)[0];
    const edit = await vscode.window.showInformationMessage(
      "Scaffold complete. Edit the first agent config?",
      "Edit",
      "Later",
    );
    if (edit === "Edit" && first) {
      await openAgentConfigEditor(context, first);
    }
  });

  register("kuibysheff.syncAcp", async () => {
    try {
      const result = await syncAcpAgents();
      tree.refresh();
      vscode.window.showInformationMessage(
        `Synced ${result.agentCount} ACP agent(s).`,
      );
    } catch (err) {
      vscode.window.showErrorMessage(String(err));
    }
  });

  register("kuibysheff.editAgent", async (item?: unknown) => {
    const id = await pickAgentId(agentIdFromArg(item));
    if (!id) {
      return;
    }
    await openAgentConfigEditor(context, id);
  });

  register("kuibysheff.openYaml", async (item?: unknown) => {
    const root = requireWorkspace();
    if (!root) {
      return;
    }
    const id = await pickAgentId(agentIdFromArg(item));
    if (!id) {
      return;
    }
    const file = path.join(
      root,
      ".kuibysheff",
      "agents",
      id,
      "agent-config.yaml",
    );
    if (!fs.existsSync(file)) {
      vscode.window.showErrorMessage(`Missing ${file}`);
      return;
    }
    const doc = await vscode.workspace.openTextDocument(file);
    await vscode.window.showTextDocument(doc);
  });

  register("kuibysheff.check", async (item?: unknown) => {
    const root = requireWorkspace();
    if (!root) {
      return;
    }
    const id = await pickAgentId(agentIdFromArg(item));
    if (!id) {
      return;
    }
    const out = getOutput();
    out.show(true);
    out.appendLine(`> check ${id}`);
    const result = await runAgentCheck({
      workspaceRoot: root,
      agentId: id,
    });
    out.appendLine(result.stdout);
    if (result.stderr) {
      out.appendLine(result.stderr);
    }
    if (result.code === 0) {
      vscode.window.showInformationMessage(`Check OK: ${id}`);
    } else {
      vscode.window.showErrorMessage(
        `Check failed for ${id}. See Kuibysheff output.`,
      );
    }
    tree.refresh();
  });

  register("kuibysheff.prepare", async () => {
    await runPrepare({});
    tree.refresh();
  });

  register("kuibysheff.promote", async () => {
    const stage = await pickStageQuick("Promote stage");
    if (!stage) {
      return;
    }
    await runPrepare({ stage, promote: true });
    tree.refresh();
  });

  register("kuibysheff.validate", async () => {
    const stage = await pickStageQuick("Validate stage");
    if (!stage) {
      return;
    }
    await runPrepare({ stage, validate: true });
    tree.refresh();
  });

  register("kuibysheff.promoteAndValidate", async () => {
    const stage = await pickStageQuick("Promote and validate");
    if (!stage) {
      return;
    }
    await runPrepare({ stage, promote: true, validate: true });
    tree.refresh();
  });

  register("kuibysheff.approvePlan", async () => {
    await runPrepare({ approvePlan: true });
    tree.refresh();
  });

  register("kuibysheff.copyChatStarter", async () => {
    const root = requireWorkspace();
    if (!root) {
      return;
    }
    const stage = await pickStageQuick("Copy chat starter for stage");
    if (!stage) {
      return;
    }
    const starterFile = chatStarterPath(root, stage);
    let text =
      "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn.";
    if (fs.existsSync(starterFile)) {
      text = fs.readFileSync(starterFile, "utf8").trim();
    }
    await vscode.env.clipboard.writeText(text);
    const prompt = stagePromptPath(root, stage);
    vscode.window.showInformationMessage(
      `Chat starter copied. Stage prompt: ${prompt}`,
    );
  });
}

async function runPrepare(options: {
  stage?: string;
  promote?: boolean;
  validate?: boolean;
  approvePlan?: boolean;
}): Promise<void> {
  const workspaceRoot = requireWorkspace();
  if (!workspaceRoot) {
    return;
  }
  const repoRoot = await ensureRepoRoot();
  if (!repoRoot) {
    return;
  }

  const script = prepareScript(repoRoot);
  if (!fs.existsSync(script)) {
    vscode.window.showErrorMessage(`Prepare script not found: ${script}`);
    return;
  }

  const args: string[] = [
    "-ProjectRoot",
    workspaceRoot,
    "-RepoRoot",
    repoRoot,
  ];

  if (options.approvePlan) {
    args.push("-ApprovePlan");
  } else {
    let stage = options.stage;
    const isPrepareOnly =
      !options.promote && !options.validate && !options.approvePlan;

    if (!stage) {
      stage = await pickStageQuick(
        isPrepareOnly ? "Prepare stage" : "Stage",
      );
      if (!stage) {
        return;
      }
    }

    if (isPrepareOnly) {
      const issueKey = await vscode.window.showInputBox({
        title: "Issue key",
        value: getDefaultIssueKey(),
        placeHolder: "PROJ-123 (optional if TaskFile set)",
      });
      const taskFile = await vscode.window.showInputBox({
        title: "Task file (optional)",
        placeHolder: "Absolute path to tz.md / brief",
      });
      if (issueKey) {
        args.push("-IssueKey", issueKey);
      }
      if (taskFile?.trim()) {
        args.push("-TaskFile", taskFile.trim());
      }
    }

    args.push("-Stage", stage);
    if (options.promote) {
      args.push("-Promote");
    }
    if (options.validate) {
      args.push("-Validate");
    }
  }

  const out = getOutput();
  out.show(true);
  out.appendLine(`> ${path.basename(script)} ${args.join(" ")}`);
  const result = await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Kuibysheff workflow…",
      cancellable: false,
    },
    async () => runPowerShellScript(script, args, workspaceRoot),
  );
  out.appendLine(result.stdout);
  if (result.stderr) {
    out.appendLine(result.stderr);
  }

  if (result.code !== 0) {
    vscode.window.showErrorMessage(
      `Workflow failed (exit ${result.code}). See Kuibysheff output.`,
    );
    return;
  }

  if (!options.approvePlan && !options.promote && !options.validate) {
    const defaultStarter =
      "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn.";
    const copy = await vscode.window.showInformationMessage(
      "Prepare complete.",
      "Copy chat starter",
    );
    if (copy === "Copy chat starter") {
      await vscode.env.clipboard.writeText(defaultStarter);
    }
  } else {
    vscode.window.showInformationMessage("Workflow step completed.");
  }
}
