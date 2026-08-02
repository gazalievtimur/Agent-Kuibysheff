const { spawn } = require("child_process");
const path = require("path");

const root = path.resolve(__dirname, "..");
const proc = spawn("node", [path.join(root, "mcp-aoc-tasks.js")], {
  env: process.env,
  stdio: ["pipe", "pipe", "inherit"],
});

let buf = "";
let step = 0;

proc.stdout.setEncoding("utf8");
proc.stdout.on("data", (chunk) => {
  buf += chunk;
  for (;;) {
    const newline = buf.indexOf("\n");
    if (newline < 0) return;
    const line = buf.slice(0, newline).replace(/\r$/, "");
    buf = buf.slice(newline + 1);
    if (!line.trim()) continue;
    onMessage(JSON.parse(line));
  }
});

function send(obj) {
  proc.stdin.write(`${JSON.stringify(obj)}\n`);
}

function onMessage(msg) {
  if (step === 0) {
    send({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "aoc_get_task", arguments: { task_id: "2024-01-1" } },
    });
    step = 1;
    return;
  }
  if (step === 1) {
    const obj = JSON.parse(msg.result.content[0].text);
    if (Object.prototype.hasOwnProperty.call(obj, "expected")) {
      console.error("FAIL: expected leaked from aoc_get_task");
      process.exit(1);
    }
    console.log(`task_ok id=${obj.id} text_len=${obj.text.length}`);
    send({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: { name: "aoc_get_input", arguments: { task_id: "2024-01-1" } },
    });
    step = 2;
    return;
  }
  const obj = JSON.parse(msg.result.content[0].text);
  const lines = obj.input.split(/\r?\n/).filter(Boolean).length;
  console.log(`input_ok lines=${lines}`);
  proc.kill();
  process.exit(0);
}

send({
  jsonrpc: "2.0",
  id: 1,
  method: "initialize",
  params: {
    protocolVersion: "2024-11-05",
    capabilities: {},
    clientInfo: { name: "aoc-smoke", version: "0" },
  },
});
