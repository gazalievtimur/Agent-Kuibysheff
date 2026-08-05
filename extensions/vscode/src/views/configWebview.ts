import * as fs from "fs";
import * as vscode from "vscode";
import { syncAcpAgents } from "../acp/syncAgents";
import {
  agentConfigPath,
  getWorkspaceRoot,
} from "../paths";
import { runAgentCheck } from "../process/runBinary";
import {
  applyEditable,
  BUILTIN_TOOLS,
  EditableAgentConfig,
  loadAppConfig,
  saveAppConfig,
  toEditable,
} from "../yaml/agentConfig";

const panels = new Map<string, vscode.WebviewPanel>();

export async function openAgentConfigEditor(
  context: vscode.ExtensionContext,
  agentId: string,
): Promise<void> {
  const workspaceRoot = getWorkspaceRoot();
  if (!workspaceRoot) {
    vscode.window.showErrorMessage("No workspace folder open");
    return;
  }

  const configPath = agentConfigPath(workspaceRoot, agentId);
  if (!fs.existsSync(configPath)) {
    vscode.window.showErrorMessage(`Missing config: ${configPath}`);
    return;
  }

  const existing = panels.get(agentId);
  if (existing) {
    existing.reveal(vscode.ViewColumn.One);
    return;
  }

  const panel = vscode.window.createWebviewPanel(
    "kuibysheffAgentConfig",
    `Kuibysheff: ${agentId}`,
    vscode.ViewColumn.One,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "media")],
    },
  );
  panels.set(agentId, panel);
  panel.onDidDispose(() => panels.delete(agentId));

  const cssUri = panel.webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "media", "webview.css"),
  );

  let appConfig = loadAppConfig(configPath);
  let editable = toEditable(appConfig);

  const postConfig = (): void => {
    panel.webview.postMessage({ type: "config", agentId, config: editable });
  };

  panel.webview.html = getHtml(panel.webview, cssUri, agentId);

  panel.webview.onDidReceiveMessage(async (msg: {
    type: string;
    config?: EditableAgentConfig;
  }) => {
    try {
      switch (msg.type) {
        case "ready":
          postConfig();
          break;
        case "save": {
          if (!msg.config) {
            return;
          }
          appConfig = applyEditable(appConfig, msg.config);
          saveAppConfig(configPath, appConfig);
          editable = toEditable(appConfig);
          panel.webview.postMessage({
            type: "status",
            ok: true,
            text: `Saved ${configPath}`,
          });
          postConfig();
          break;
        }
        case "validate": {
          if (msg.config) {
            appConfig = applyEditable(appConfig, msg.config);
            saveAppConfig(configPath, appConfig);
            editable = toEditable(appConfig);
          }
          const result = await runAgentCheck({
            workspaceRoot,
            configRelative: `agents/${agentId}/agent-config.yaml`,
            settingsDirRelative: `agents/${agentId}`,
          });
          const text = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
          panel.webview.postMessage({
            type: "status",
            ok: result.code === 0,
            text: text || (result.code === 0 ? "Check OK" : `Exit ${result.code}`),
          });
          break;
        }
        case "syncAcp": {
          const result = await syncAcpAgents(workspaceRoot);
          panel.webview.postMessage({
            type: "status",
            ok: true,
            text: `Synced ${result.agentCount} ACP agents → ${result.path}`,
          });
          break;
        }
        case "openYaml": {
          const doc = await vscode.workspace.openTextDocument(configPath);
          await vscode.window.showTextDocument(doc, vscode.ViewColumn.Beside);
          break;
        }
        default:
          break;
      }
    } catch (err) {
      panel.webview.postMessage({
        type: "status",
        ok: false,
        text: String(err),
      });
    }
  });

  // Allow reload if file changed externally after open
  const watcher = vscode.workspace.createFileSystemWatcher(configPath);
  watcher.onDidChange(() => {
    try {
      appConfig = loadAppConfig(configPath);
      editable = toEditable(appConfig);
      postConfig();
    } catch {
      // ignore transient parse errors while saving
    }
  });
  panel.onDidDispose(() => watcher.dispose());
}

function getHtml(
  webview: vscode.Webview,
  cssUri: vscode.Uri,
  agentId: string,
): string {
  const nonce = getNonce();
  const builtinsJson = JSON.stringify([...BUILTIN_TOOLS]);
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta http-equiv="Content-Security-Policy"
    content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <link rel="stylesheet" href="${cssUri}" />
  <title>${agentId}</title>
</head>
<body>
  <h1 id="title">${escapeHtml(agentId)}</h1>
  <p class="subtitle">Edit agent parameters. Secrets stay in environment variables (<code>api_key_env</code>), not in YAML.</p>

  <div class="toolbar">
    <button id="btnSave">Save</button>
    <button id="btnValidate" class="secondary">Validate (check)</button>
    <button id="btnSync" class="secondary">Sync ACP</button>
    <button id="btnYaml" class="secondary">Open YAML</button>
  </div>
  <div id="status" class="status"></div>

  <section>
    <h2>Provider</h2>
    <div class="grid">
      <label>Base URL <input id="base_url" type="text" /></label>
      <label>Model <input id="model" type="text" /></label>
      <label>API key env <input id="api_key_env" type="text" /></label>
      <label>Timeout ms <input id="timeout_ms" type="number" /></label>
      <label>Max retries <input id="max_retries" type="number" /></label>
      <label>Retry base delay ms <input id="retry_base_delay_ms" type="number" /></label>
      <label>History max tail messages <input id="max_tail_messages" type="number" /></label>
      <label>History max chars <input id="max_chars" type="number" /></label>
    </div>
    <p class="hint">Set the named env var in your user/machine environment before chatting with ACP agents.</p>
  </section>

  <section>
    <h2>Limits</h2>
    <div class="grid">
      <label>Max iterations <input id="max_iterations" type="number" /></label>
      <label>Max tokens <input id="max_tokens" type="number" /></label>
      <label>Max duration sec <input id="max_duration_sec" type="number" /></label>
    </div>
  </section>

  <section>
    <h2>Logging</h2>
    <div class="grid">
      <label class="checkbox"><input id="enable_ai_log" type="checkbox" /> AI log</label>
      <label class="checkbox"><input id="enable_mcp_log" type="checkbox" /> MCP log</label>
      <label class="checkbox"><input id="enable_chat_history" type="checkbox" /> Chat history</label>
    </div>
  </section>

  <section>
    <h2>MCP servers</h2>
    <div id="mcpList"></div>
    <button id="btnAddMcp" class="secondary" type="button">Add MCP</button>
  </section>

  <section>
    <h2>Access</h2>
    <h3 style="font-size:0.9rem;margin:0 0 8px">Builtins</h3>
    <div id="builtins" class="builtins"></div>
    <div class="grid" style="margin-top:12px">
      <label class="grid-full">Home read (comma-separated)
        <input id="home_read" type="text" />
      </label>
      <label class="grid-full">Home write (comma-separated)
        <input id="home_write" type="text" />
      </label>
      <label class="grid-full">Workspace root
        <input id="workspace_root" type="text" placeholder="../../../src/cf" />
      </label>
      <label class="grid-full">Workspace read (comma-separated)
        <input id="workspace_read" type="text" />
      </label>
    </div>
    <h3 style="font-size:0.9rem;margin:16px 0 8px">Run programs</h3>
    <div id="programs"></div>
    <button id="btnAddProgram" class="secondary" type="button">Add program</button>
  </section>

  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    const BUILTINS = ${builtinsJson};
    let state = null;

    const $ = (id) => document.getElementById(id);

    function showStatus(ok, text) {
      const el = $("status");
      el.textContent = text || "";
      el.className = "status visible " + (ok ? "ok" : "err");
    }

    function csv(list) {
      return (list || []).join(", ");
    }
    function splitCsv(text) {
      return text.split(",").map((s) => s.trim()).filter(Boolean);
    }

    function parseKvLines(text) {
      const out = {};
      for (const line of (text || "").split("\\n")) {
        const t = line.trim();
        if (!t) continue;
        const i = t.indexOf("=");
        if (i <= 0) continue;
        out[t.slice(0, i).trim()] = t.slice(i + 1).trim();
      }
      return out;
    }
    function kvLines(obj) {
      return Object.entries(obj || {}).map(([k, v]) => k + "=" + v).join("\\n");
    }

    function renderBuiltins() {
      const box = $("builtins");
      box.innerHTML = "";
      const selected = new Set((state && state.access.builtins) || []);
      for (const name of BUILTINS) {
        const label = document.createElement("label");
        label.className = "checkbox";
        const input = document.createElement("input");
        input.type = "checkbox";
        input.checked = selected.has(name);
        input.dataset.builtin = name;
        label.appendChild(input);
        label.appendChild(document.createTextNode(" " + name));
        box.appendChild(label);
      }
    }

    function renderMcp() {
      const box = $("mcpList");
      box.innerHTML = "";
      const servers = (state && state.mcp) || [];
      servers.forEach((s, idx) => {
        const isHttp = s.transport === "http" || (s.url && !s.command);
        const card = document.createElement("div");
        card.className = "mcp-card";
        card.innerHTML =
          "<header><strong>MCP #" + (idx + 1) + "</strong>" +
          "<button type='button' class='secondary btn-remove-mcp' data-idx='" + idx + "'>Remove</button></header>" +
          "<div class='grid'>" +
          "<label>Name <input data-f='name' data-idx='" + idx + "' type='text' value='" + esc(s.name || "") + "' /></label>" +
          "<label>Transport <select data-f='transport' data-idx='" + idx + "'>" +
          "<option value='stdio'" + (!isHttp ? " selected" : "") + ">stdio</option>" +
          "<option value='http'" + (isHttp ? " selected" : "") + ">http</option></select></label>" +
          "<label class='stdio'>Command <input data-f='command' data-idx='" + idx + "' type='text' value='" + esc(s.command || "") + "' /></label>" +
          "<label class='stdio'>Args (one per line)<textarea data-f='args' data-idx='" + idx + "'>" + esc((s.args || []).join("\\n")) + "</textarea></label>" +
          "<label class='stdio grid-full'>Env (KEY=value per line)<textarea data-f='env' data-idx='" + idx + "'>" + esc(kvLines(s.env)) + "</textarea></label>" +
          "<label class='http grid-full'>URL <input data-f='url' data-idx='" + idx + "' type='text' value='" + esc(s.url || "") + "' /></label>" +
          "<label class='http grid-full'>Headers (KEY=value per line)<textarea data-f='headers' data-idx='" + idx + "'>" + esc(kvLines(s.headers)) + "</textarea></label>" +
          "<label>Timeout ms <input data-f='timeout_ms' data-idx='" + idx + "' type='number' value='" + (s.timeout_ms ?? 20000) + "' /></label>" +
          "</div>";
        box.appendChild(card);
        toggleMcpFields(card, isHttp);
      });
      box.querySelectorAll(".btn-remove-mcp").forEach((btn) => {
        btn.addEventListener("click", () => {
          collectFromDom();
          state.mcp.splice(Number(btn.dataset.idx), 1);
          renderMcp();
        });
      });
      box.querySelectorAll("select[data-f='transport']").forEach((sel) => {
        sel.addEventListener("change", () => {
          const card = sel.closest(".mcp-card");
          toggleMcpFields(card, sel.value === "http");
        });
      });
    }

    function toggleMcpFields(card, isHttp) {
      card.querySelectorAll(".stdio").forEach((el) => { el.style.display = isHttp ? "none" : ""; });
      card.querySelectorAll(".http").forEach((el) => { el.style.display = isHttp ? "" : "none"; });
    }

    function renderPrograms() {
      const box = $("programs");
      box.innerHTML = "";
      const programs = (state && state.access.programs) || [];
      programs.forEach((p, idx) => {
        const card = document.createElement("div");
        card.className = "program-card";
        card.innerHTML =
          "<header><strong>Program #" + (idx + 1) + "</strong>" +
          "<button type='button' class='secondary btn-remove-prog' data-idx='" + idx + "'>Remove</button></header>" +
          "<div class='grid'>" +
          "<label>Name <input data-pf='name' data-idx='" + idx + "' type='text' value='" + esc(p.name || "") + "' /></label>" +
          "<label>Executable <input data-pf='executable' data-idx='" + idx + "' type='text' value='" + esc(p.executable || "") + "' /></label>" +
          "<label class='grid-full'>Runtime read roots (comma-separated)" +
          "<input data-pf='runtime_read_roots' data-idx='" + idx + "' type='text' value='" + esc(csv(p.runtime_read_roots)) + "' /></label>" +
          "<label class='grid-full'>Inherit env (comma-separated)" +
          "<input data-pf='inherit_env' data-idx='" + idx + "' type='text' value='" + esc(csv(p.inherit_env)) + "' /></label>" +
          "<label class='checkbox'><input data-pf='allow_children' data-idx='" + idx + "' type='checkbox'" +
          (p.allow_children ? " checked" : "") + " /> Allow children</label>" +
          "</div>";
        box.appendChild(card);
      });
      box.querySelectorAll(".btn-remove-prog").forEach((btn) => {
        btn.addEventListener("click", () => {
          collectFromDom();
          state.access.programs.splice(Number(btn.dataset.idx), 1);
          renderPrograms();
        });
      });
    }

    function esc(s) {
      return String(s)
        .replace(/&/g, "&amp;")
        .replace(/"/g, "&quot;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;");
    }

    function fillForm() {
      if (!state) return;
      $("base_url").value = state.provider.base_url;
      $("model").value = state.provider.model;
      $("api_key_env").value = state.provider.api_key_env;
      $("timeout_ms").value = state.provider.timeout_ms;
      $("max_retries").value = state.provider.max_retries;
      $("retry_base_delay_ms").value = state.provider.retry_base_delay_ms;
      $("max_tail_messages").value = state.provider.history.max_tail_messages;
      $("max_chars").value = state.provider.history.max_chars;
      $("max_iterations").value = state.limits.max_iterations;
      $("max_tokens").value = state.limits.max_tokens;
      $("max_duration_sec").value = state.limits.max_duration_sec;
      $("enable_ai_log").checked = !!state.logging.enable_ai_log;
      $("enable_mcp_log").checked = !!state.logging.enable_mcp_log;
      $("enable_chat_history").checked = !!state.logging.enable_chat_history;
      $("home_read").value = csv(state.access.homeRead);
      $("home_write").value = csv(state.access.homeWrite);
      $("workspace_root").value = state.access.workspaceRoot || "";
      $("workspace_read").value = csv(state.access.workspaceRead);
      renderBuiltins();
      renderMcp();
      renderPrograms();
    }

    function collectFromDom() {
      if (!state) return state;
      state.provider.base_url = $("base_url").value.trim();
      state.provider.model = $("model").value.trim();
      state.provider.api_key_env = $("api_key_env").value.trim();
      state.provider.timeout_ms = Number($("timeout_ms").value) || 60000;
      state.provider.max_retries = Number($("max_retries").value) || 3;
      state.provider.retry_base_delay_ms = Number($("retry_base_delay_ms").value) || 500;
      state.provider.history.max_tail_messages = Number($("max_tail_messages").value) || 30;
      state.provider.history.max_chars = Number($("max_chars").value) || 200000;
      state.limits.max_iterations = Number($("max_iterations").value) || 10;
      state.limits.max_tokens = Number($("max_tokens").value) || 15000;
      state.limits.max_duration_sec = Number($("max_duration_sec").value) || 120;
      state.logging.enable_ai_log = $("enable_ai_log").checked;
      state.logging.enable_mcp_log = $("enable_mcp_log").checked;
      state.logging.enable_chat_history = $("enable_chat_history").checked;
      state.access.builtins = [...document.querySelectorAll("#builtins input[type=checkbox]")]
        .filter((el) => el.checked)
        .map((el) => el.dataset.builtin);
      state.access.homeRead = splitCsv($("home_read").value);
      state.access.homeWrite = splitCsv($("home_write").value);
      state.access.workspaceRoot = $("workspace_root").value.trim();
      state.access.workspaceRead = splitCsv($("workspace_read").value);

      const mcp = [];
      const indices = new Set(
        [...document.querySelectorAll("#mcpList [data-idx]")].map((el) => Number(el.dataset.idx))
      );
      for (const idx of [...indices].sort((a, b) => a - b)) {
        const get = (f) => document.querySelector("#mcpList [data-f='" + f + "'][data-idx='" + idx + "']");
        const transport = get("transport").value;
        const entry = {
          name: get("name").value.trim() || ("mcp-" + idx),
          transport,
          timeout_ms: Number(get("timeout_ms").value) || 20000,
        };
        if (transport === "http") {
          entry.url = get("url").value.trim();
          entry.headers = parseKvLines(get("headers").value);
        } else {
          entry.command = get("command").value.trim();
          entry.args = get("args").value.split("\\n").map((s) => s.trimEnd()).filter((s) => s.length);
          entry.env = parseKvLines(get("env").value);
        }
        mcp.push(entry);
      }
      state.mcp = mcp;

      const programs = [];
      const pidx = new Set(
        [...document.querySelectorAll("#programs [data-idx]")].map((el) => Number(el.dataset.idx))
      );
      for (const idx of [...pidx].sort((a, b) => a - b)) {
        const get = (f) => document.querySelector("#programs [data-pf='" + f + "'][data-idx='" + idx + "']");
        programs.push({
          name: get("name").value.trim(),
          executable: get("executable").value.trim(),
          runtime_read_roots: splitCsv(get("runtime_read_roots").value),
          inherit_env: splitCsv(get("inherit_env").value),
          allow_children: get("allow_children").checked,
        });
      }
      state.access.programs = programs;
      return state;
    }

    $("btnSave").addEventListener("click", () => {
      vscode.postMessage({ type: "save", config: collectFromDom() });
    });
    $("btnValidate").addEventListener("click", () => {
      vscode.postMessage({ type: "validate", config: collectFromDom() });
    });
    $("btnSync").addEventListener("click", () => {
      vscode.postMessage({ type: "syncAcp" });
    });
    $("btnYaml").addEventListener("click", () => {
      vscode.postMessage({ type: "openYaml" });
    });
    $("btnAddMcp").addEventListener("click", () => {
      collectFromDom();
      state.mcp.push({ name: "new-mcp", transport: "stdio", command: "", args: [], timeout_ms: 20000 });
      renderMcp();
    });
    $("btnAddProgram").addEventListener("click", () => {
      collectFromDom();
      state.access.programs.push({
        name: "",
        executable: "",
        runtime_read_roots: [],
        inherit_env: [],
        allow_children: false,
      });
      renderPrograms();
    });

    window.addEventListener("message", (event) => {
      const msg = event.data;
      if (msg.type === "config") {
        state = msg.config;
        $("title").textContent = msg.agentId;
        fillForm();
      } else if (msg.type === "status") {
        showStatus(!!msg.ok, msg.text || "");
      }
    });

    vscode.postMessage({ type: "ready" });
  </script>
</body>
</html>`;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function getNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let text = "";
  for (let i = 0; i < 32; i++) {
    text += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return text;
}
