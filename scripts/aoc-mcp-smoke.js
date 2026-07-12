const { spawn } = require("child_process");
const path = require("path");

const root = path.resolve(__dirname, "..");
const proc = spawn("node", [path.join(root, "mcp-aoc-tasks.js")], {
  env: process.env,
  stdio: ["pipe", "pipe", "inherit"],
});

let buf = Buffer.alloc(0);
let step = 0;

proc.stdout.on("data", (chunk) => {
  buf = Buffer.concat([buf, chunk]);
  for (;;) {
    const headerEnd = buf.indexOf("\r\n\r\n");
    if (headerEnd < 0) return;
    const match = /content-length:\s*(\d+)/i.exec(buf.slice(0, headerEnd).toString());
    if (!match) {
      buf = Buffer.alloc(0);
      return;
    }
    const len = Number(match[1]);
    const end = headerEnd + 4 + len;
    if (buf.length < end) return;
    const msg = JSON.parse(buf.slice(headerEnd + 4, end).toString());
    buf = buf.slice(end);
    onMessage(msg);
  }
});

function send(obj) {
  const body = Buffer.from(JSON.stringify(obj), "utf8");
  proc.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
  proc.stdin.write(body);
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
