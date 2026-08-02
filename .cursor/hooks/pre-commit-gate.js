#!/usr/bin/env node
/**
 * Cursor beforeShellExecution hook for `git commit`:
 * - Refuse `--no-verify` unless SKIP_PRECOMMIT=1
 * - Require `core.hooksPath=.githooks` so the git pre-commit gate runs once
 *
 * The heavy CI-parity work lives in `.githooks/pre-commit` →
 * `scripts/pre-commit-gate.*` (fmt / clippy / test / miri).
 */
"use strict";

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

function readStdin() {
  try {
    return fs.readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

function allow() {
  process.stdout.write(JSON.stringify({ permission: "allow" }));
}

function deny(message) {
  process.stdout.write(
    JSON.stringify({
      permission: "deny",
      user_message: message,
      agent_message: message,
    }),
  );
}

function isGitCommit(command) {
  const normalized = command.replace(/\r\n/g, "\n");
  return /(?:^|[;&|\n]|&&|\|\|)\s*(?:[^\s]*[/\\])?git(?:\.exe)?\s+commit\b/i.test(
    normalized,
  );
}

function hasNoVerify(command) {
  return /(?:^|\s)(--no-verify|-n)(?=\s|$)/.test(command);
}

function hooksPathConfigured(root) {
  const result = spawnSync("git", ["config", "--get", "core.hooksPath"], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.status !== 0) {
    return false;
  }
  const value = String(result.stdout || "")
    .trim()
    .replace(/\\/g, "/");
  return value === ".githooks" || value.endsWith("/.githooks");
}

function main() {
  const raw = readStdin();
  let input = {};
  try {
    input = raw.trim() ? JSON.parse(raw) : {};
  } catch {
    allow();
    return;
  }

  const command = String(input.command || "");
  if (!isGitCommit(command)) {
    allow();
    return;
  }

  if (process.env.SKIP_PRECOMMIT === "1") {
    allow();
    return;
  }

  if (hasNoVerify(command)) {
    deny(
      "git commit --no-verify is blocked. Fix fmt/clippy/test/miri failures, or set SKIP_PRECOMMIT=1 for an emergency bypass.",
    );
    return;
  }

  const root = path.resolve(__dirname, "..", "..");
  if (!hooksPathConfigured(root)) {
    const install =
      process.platform === "win32"
        ? ".\\scripts\\install-git-hooks.ps1"
        : "./scripts/install-git-hooks.sh";
    deny(
      `git core.hooksPath is not set to .githooks. Run \`${install}\` once, then commit again.`,
    );
    return;
  }

  allow();
}

main();
