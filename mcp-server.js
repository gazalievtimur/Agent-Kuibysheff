#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const ROOT_DIR = process.cwd();
const SKIP_DIRS = new Set([".git", "node_modules", "target", "logs", ".cursor"]);
const ALLOWED_EXT = new Set([".md", ".rs", ".toml", ".yaml", ".yml", ".json", ".txt"]);

const TOOLS = [
  {
    name: "search_docs",
    description: "Search text in project docs and source files.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search phrase" },
        max_results: { type: "integer", minimum: 1, maximum: 20, default: 8 },
      },
      required: ["query"],
    },
  },
  {
    name: "read_file",
    description: "Read a text file relative to project root.",
    inputSchema: {
      type: "object",
      properties: {
        path: { type: "string", description: "Relative path from repository root" },
        max_chars: { type: "integer", minimum: 100, maximum: 200000, default: 6000 },
      },
      required: ["path"],
    },
  },
];

let buffered = Buffer.alloc(0);
process.stdin.on("data", (chunk) => {
  buffered = Buffer.concat([buffered, chunk]);
  processFrames();
});

process.stdin.on("error", () => {
  process.exit(1);
});

function processFrames() {
  for (;;) {
    const headerEnd = buffered.indexOf("\r\n\r\n");
    if (headerEnd === -1) {
      return;
    }

    const headerText = buffered.slice(0, headerEnd).toString("utf8");
    const match = /content-length:\s*(\d+)/i.exec(headerText);
    if (!match) {
      buffered = Buffer.alloc(0);
      return;
    }

    const bodyLen = Number.parseInt(match[1], 10);
    const messageEnd = headerEnd + 4 + bodyLen;
    if (buffered.length < messageEnd) {
      return;
    }

    const body = buffered.slice(headerEnd + 4, messageEnd).toString("utf8");
    buffered = buffered.slice(messageEnd);

    let message;
    try {
      message = JSON.parse(body);
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
            name: "local-demo-mcp",
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
  if (name === "search_docs") {
    const query = String(args.query ?? "").trim();
    const maxResults = clampInt(args.max_results, 8, 1, 20);
    if (!query) {
      return errorToolResult("`query` must not be empty");
    }
    const matches = searchDocs(query, maxResults);
    return successToolResult({
      query,
      total_matches: matches.length,
      matches,
    });
  }

  if (name === "read_file") {
    const relPath = String(args.path ?? "").trim();
    const maxChars = clampInt(args.max_chars, 6000, 100, 200000);
    if (!relPath) {
      return errorToolResult("`path` must not be empty");
    }
    const fullPath = resolveInsideRoot(relPath);
    if (!fullPath) {
      return errorToolResult("path escapes repository root");
    }
    let text;
    try {
      text = fs.readFileSync(fullPath, "utf8");
    } catch (err) {
      return errorToolResult(`failed to read file: ${err instanceof Error ? err.message : String(err)}`);
    }
    if (text.length > maxChars) {
      text = `${text.slice(0, maxChars)}\n...[truncated]`;
    }
    return successToolResult({
      path: path.relative(ROOT_DIR, fullPath).replaceAll("\\", "/"),
      content: text,
    });
  }

  return errorToolResult(`unknown tool: ${String(name)}`);
}

function searchDocs(query, maxResults) {
  const lowered = query.toLowerCase();
  const files = collectFiles(ROOT_DIR, 0, 600);
  const out = [];

  for (const filePath of files) {
    if (out.length >= maxResults) {
      break;
    }
    let text;
    try {
      text = fs.readFileSync(filePath, "utf8");
    } catch {
      continue;
    }
    if (text.length > 250000) {
      continue;
    }

    const lines = text.split(/\r?\n/);
    for (let idx = 0; idx < lines.length; idx += 1) {
      const line = lines[idx];
      if (line.toLowerCase().includes(lowered)) {
        out.push({
          file: path.relative(ROOT_DIR, filePath).replaceAll("\\", "/"),
          line: idx + 1,
          snippet: line.trim().slice(0, 300),
        });
        break;
      }
    }
  }
  return out;
}

function collectFiles(dir, depth, maxFiles) {
  if (depth > 8 || maxFiles <= 0) {
    return [];
  }
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }

  const files = [];
  for (const entry of entries) {
    if (files.length >= maxFiles) {
      break;
    }

    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) {
        files.push(...collectFiles(full, depth + 1, maxFiles - files.length));
      }
      continue;
    }
    if (!entry.isFile()) {
      continue;
    }
    const ext = path.extname(entry.name).toLowerCase();
    if (ALLOWED_EXT.has(ext)) {
      files.push(full);
    }
  }
  return files;
}

function resolveInsideRoot(relPath) {
  const fullPath = path.resolve(ROOT_DIR, relPath);
  const rel = path.relative(ROOT_DIR, fullPath);
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    return null;
  }
  return fullPath;
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
  const body = Buffer.from(JSON.stringify(msg), "utf8");
  const header = `Content-Length: ${body.length}\r\n\r\n`;
  process.stdout.write(header, "utf8");
  process.stdout.write(body);
}
