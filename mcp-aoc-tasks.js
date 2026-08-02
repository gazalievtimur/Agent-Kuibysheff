#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

function readArg(name) {
  const prefix = `--${name}=`;
  for (const arg of process.argv.slice(2)) {
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
  }
  return null;
}

const BANK_DIR = path.resolve(
  readArg("bank-dir") ||
    process.env.AOC_BANK_DIR ||
    path.join(process.cwd(), "local", "aoc-bank")
);

function homeDir() {
  const fromArg = readArg("home-dir");
  const raw = fromArg || process.env.AOC_HOME_DIR || null;
  return raw ? path.resolve(raw) : null;
}

const TOOLS = [
  {
    name: "aoc_get_task",
    description:
      "Load an Advent of Code style task by task_id or url. Returns id, title, url, and text. Never returns expected answers.",
    inputSchema: {
      type: "object",
      properties: {
        task_id: { type: "string", description: "Task id, e.g. 2024-01-1" },
        url: { type: "string", description: "Task URL when id is unknown" },
      },
    },
  },
  {
    name: "aoc_get_input",
    description:
      "Load puzzle input for a task by task_id. When AOC_HOME_DIR is set, writes input.txt into that home directory and returns path metadata instead of the full payload (keeps model context small).",
    inputSchema: {
      type: "object",
      properties: {
        task_id: { type: "string", description: "Task id, e.g. 2024-01-1" },
      },
      required: ["task_id"],
    },
  },
  {
    name: "aoc_list_tasks",
    description: "List available tasks (id, title, url).",
    inputSchema: {
      type: "object",
      properties: {
        limit: { type: "integer", minimum: 1, maximum: 200, default: 50 },
      },
    },
  },
  {
    name: "aoc_search_tasks",
    description: "Search tasks by substring in id, title, url, or text.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search phrase" },
        limit: { type: "integer", minimum: 1, maximum: 50, default: 10 },
      },
      required: ["query"],
    },
  },
];

let buffered = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffered += chunk;
  processFrames();
});

process.stdin.on("error", () => {
  process.exit(1);
});

function processFrames() {
  for (;;) {
    const newline = buffered.indexOf("\n");
    if (newline === -1) {
      return;
    }

    const line = buffered.slice(0, newline).replace(/\r$/, "");
    buffered = buffered.slice(newline + 1);
    if (!line.trim()) {
      continue;
    }

    let message;
    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }
    void handleMessage(message);
  }
}

async function handleMessage(msg) {
  const method = msg?.method;
  const id = msg?.id;

  if (method === "notifications/initialized") {
    return;
  }

  if (typeof id === "undefined") {
    return;
  }

  try {
    switch (method) {
      case "initialize":
        sendResponse(id, {
          protocolVersion: "2024-11-05",
          capabilities: {
            tools: {},
          },
          serverInfo: {
            name: "aoc-tasks-mcp",
            version: "0.1.0",
          },
        });
        return;
      case "tools/list":
        sendResponse(id, { tools: TOOLS });
        return;
      case "tools/call": {
        const name = msg?.params?.name;
        const args = msg?.params?.arguments ?? {};
        const result = await callTool(name, args);
        sendResponse(id, result);
        return;
      }
      default:
        sendError(id, -32601, `method not found: ${String(method)}`);
    }
  } catch (err) {
    sendError(id, -32000, err instanceof Error ? err.message : String(err));
  }
}

async function callTool(name, args) {
  if (name === "aoc_get_task") {
    const taskId = String(args.task_id ?? "").trim();
    const url = String(args.url ?? "").trim();
    if (!taskId && !url) {
      return errorToolResult("provide `task_id` or `url`");
    }
    const task = taskId ? loadTaskById(taskId) : loadTaskByUrl(url);
    if (!task) {
      return errorToolResult(
        taskId ? `task not found: ${taskId}` : `task not found for url: ${url}`
      );
    }
    return successToolResult(publicTask(task));
  }

  if (name === "aoc_get_input") {
    const taskId = String(args.task_id ?? "").trim();
    if (!taskId) {
      return errorToolResult("`task_id` must not be empty");
    }
    const task = loadTaskById(taskId);
    if (!task) {
      return errorToolResult(`task not found: ${taskId}`);
    }
    const input = String(task.input ?? "");
    const home = homeDir();
    if (home) {
      try {
        fs.mkdirSync(home, { recursive: true });
        const dest = path.join(home, "input.txt");
        const resolved = path.resolve(dest);
        if (resolved !== dest || !resolved.startsWith(home)) {
          return errorToolResult("refusing to write input outside AOC_HOME_DIR");
        }
        fs.writeFileSync(dest, input.endsWith("\n") ? input : `${input}\n`, "utf8");
        return successToolResult({
          id: task.id,
          path: "input.txt",
          bytes: Buffer.byteLength(input, "utf8"),
          lines: input.split(/\r?\n/).filter((line) => line.length > 0).length,
          note: "Puzzle input written to home/input.txt. Read it from disk in your solution; full input is not inlined here.",
        });
      } catch (err) {
        return errorToolResult(
          `failed to write input.txt: ${err instanceof Error ? err.message : String(err)}`
        );
      }
    }
    return successToolResult({
      id: task.id,
      input,
    });
  }

  if (name === "aoc_list_tasks") {
    const limit = clampInt(args.limit, 50, 1, 200);
    const tasks = loadAllTasks()
      .slice(0, limit)
      .map((task) => ({
        id: task.id,
        title: task.title ?? "",
        url: task.url ?? "",
      }));
    return successToolResult({
      bank_dir: BANK_DIR,
      total: tasks.length,
      tasks,
    });
  }

  if (name === "aoc_search_tasks") {
    const query = String(args.query ?? "").trim().toLowerCase();
    const limit = clampInt(args.limit, 10, 1, 50);
    if (!query) {
      return errorToolResult("`query` must not be empty");
    }
    const matches = loadAllTasks()
      .filter((task) => {
        const haystack = [
          task.id,
          task.title ?? "",
          task.url ?? "",
          task.text ?? "",
        ]
          .join("\n")
          .toLowerCase();
        return haystack.includes(query);
      })
      .slice(0, limit)
      .map((task) => ({
        id: task.id,
        title: task.title ?? "",
        url: task.url ?? "",
      }));
    return successToolResult({
      query,
      total_matches: matches.length,
      matches,
    });
  }

  return errorToolResult(`unknown tool: ${String(name)}`);
}

function publicTask(task) {
  return {
    id: task.id,
    title: task.title ?? "",
    url: task.url ?? "",
    text: String(task.text ?? ""),
  };
}

function loadTaskById(taskId) {
  return loadAllTasks().find((task) => task.id === taskId) ?? null;
}

function loadTaskByUrl(url) {
  return loadAllTasks().find((task) => (task.url ?? "") === url) ?? null;
}

function loadAllTasks() {
  if (!fs.existsSync(BANK_DIR) || !fs.statSync(BANK_DIR).isDirectory()) {
    throw new Error(`AOC bank directory not found: ${BANK_DIR}`);
  }

  const files = fs
    .readdirSync(BANK_DIR)
    .filter((name) => name.toLowerCase().endsWith(".json"))
    .sort();

  const tasks = [];
  for (const fileName of files) {
    const fullPath = path.join(BANK_DIR, fileName);
    let raw;
    try {
      raw = fs.readFileSync(fullPath, "utf8");
    } catch {
      continue;
    }

    let parsed;
    try {
      parsed = JSON.parse(raw);
    } catch (err) {
      throw new Error(
        `invalid JSON in ${fileName}: ${err instanceof Error ? err.message : String(err)}`
      );
    }

    const id = String(parsed.id ?? "").trim();
    if (!id) {
      throw new Error(`task file ${fileName} is missing required field id`);
    }
    if (typeof parsed.text !== "string") {
      throw new Error(`task ${id} is missing required string field text`);
    }
    if (typeof parsed.input !== "string") {
      throw new Error(`task ${id} is missing required string field input`);
    }

    // Strip expected before any tool response path can see it on the object
    // used for public views; keep it only on the private loaded record for
    // harness code that reads files directly — MCP never returns it.
    tasks.push({
      id,
      title: parsed.title,
      url: parsed.url,
      text: parsed.text,
      input: parsed.input,
    });
  }

  tasks.sort((a, b) => a.id.localeCompare(b.id));
  return tasks;
}

function clampInt(value, fallback, min, max) {
  const n = Number.parseInt(String(value ?? fallback), 10);
  if (Number.isNaN(n)) {
    return fallback;
  }
  return Math.max(min, Math.min(max, n));
}

function successToolResult(payload) {
  return {
    content: [
      {
        type: "text",
        text: JSON.stringify(payload),
      },
    ],
  };
}

function errorToolResult(message) {
  return {
    isError: true,
    content: [
      {
        type: "text",
        text: message,
      },
    ],
  };
}

function sendResponse(id, result) {
  writeMessage({
    jsonrpc: "2.0",
    id,
    result,
  });
}

function sendError(id, code, message) {
  writeMessage({
    jsonrpc: "2.0",
    id,
    error: {
      code,
      message,
    },
  });
}

function writeMessage(msg) {
  process.stdout.write(`${JSON.stringify(msg)}\n`, "utf8");
}
