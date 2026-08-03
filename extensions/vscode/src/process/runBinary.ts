import { spawn } from "child_process";
import * as path from "path";
import { getBinaryPath } from "../settings";

export interface RunResult {
  code: number | null;
  stdout: string;
  stderr: string;
}

function runProcess(
  command: string,
  args: string[],
  cwd: string,
): Promise<RunResult> {
  return new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd,
      shell: true,
      windowsHide: true,
      env: process.env,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });
    child.on("error", (err) => {
      resolve({ code: 1, stdout, stderr: `${stderr}\n${err.message}`.trim() });
    });
    child.on("close", (code) => {
      resolve({ code, stdout, stderr });
    });
  });
}

export async function runAgentCheck(options: {
  workspaceRoot: string;
  /** Path relative to `{workspace}/.kuibyshev/`, e.g. `agents/1c-analyst/agent-config.yaml`. */
  configRelative: string;
  /** Path relative to `{workspace}/.kuibyshev/`, e.g. `agents/1c-analyst`. */
  settingsDirRelative: string;
  skipProvider?: boolean;
  skipMcp?: boolean;
}): Promise<RunResult> {
  const binary = getBinaryPath();
  const kuib = path.join(options.workspaceRoot, ".kuibyshev");
  const configAbs = path.join(kuib, options.configRelative);
  const settingsAbs = path.join(kuib, options.settingsDirRelative);
  const args = [
    "check",
    "--config",
    configAbs,
    "--settings-dir",
    settingsAbs,
  ];
  if (options.skipProvider) {
    args.push("--skip-provider");
  }
  if (options.skipMcp) {
    args.push("--skip-mcp");
  }
  return runProcess(binary, args, options.workspaceRoot);
}

export async function runPowerShellScript(
  scriptPath: string,
  scriptArgs: string[],
  cwd: string,
): Promise<RunResult> {
  const args = [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    scriptPath,
    ...scriptArgs,
  ];
  return runProcess("powershell", args, cwd);
}

export function scaffoldScript(repoRoot: string): string {
  return path.join(repoRoot, "scripts", "1c-dev-scaffold-project.ps1");
}

export function prepareScript(repoRoot: string): string {
  return path.join(repoRoot, "scripts", "1c-dev-acp-prepare.ps1");
}
