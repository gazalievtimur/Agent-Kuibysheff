import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

/** Mirrors extensions/vscode/src/paths.ts pure path builders (no vscode import). */
function kuibysheffRoot(workspaceRoot) {
  return path.join(workspaceRoot, ".kuibysheff");
}
function agentsRoot(workspaceRoot) {
  return path.join(kuibysheffRoot(workspaceRoot), "agents");
}
function agentDir(workspaceRoot, agentId) {
  return path.join(agentsRoot(workspaceRoot), agentId);
}
function agentConfigPath(workspaceRoot, agentId) {
  return path.join(agentDir(workspaceRoot, agentId), "agent-config.yaml");
}
function runsRoot(workspaceRoot) {
  return path.join(kuibysheffRoot(workspaceRoot), "runs", "vscode-active");
}
function stageHome(workspaceRoot, stage) {
  return path.join(runsRoot(workspaceRoot), `stage${stage}`, "home");
}

test("kuibysheff layout paths", () => {
  const ws = path.join("C:", "proj");
  assert.equal(kuibysheffRoot(ws), path.join(ws, ".kuibysheff"));
  assert.equal(
    agentConfigPath(ws, "1c-coder"),
    path.join(ws, ".kuibysheff", "agents", "1c-coder", "agent-config.yaml")
  );
  assert.equal(
    stageHome(ws, "3"),
    path.join(ws, ".kuibysheff", "runs", "vscode-active", "stage3", "home")
  );
});
