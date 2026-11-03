(() => {
  const STUDIO_HEADER = { "X-Kbshff-Studio": "1" };

  const SET_TOKEN = "<set>";

  const state = {
    agents: [],
    agentId: null,
    rawConfig: null,
    mcpServers: [],
    mcpProbe: { servers: [], preferredServer: null },
    mcpResult: { raw: null, view: "pretty" },
    runOutput: { raw: null, view: "pretty" },
    runEvents: { items: [], view: "pretty" },
    currentRunId: null,
    eventSource: null,
    skillBodyName: "",
  };

  const $ = (id) => document.getElementById(id);
  const lines = (text) =>
    text
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
  const joinLines = (arr) => (arr || []).join("\n");

  async function api(path, options = {}) {
    const headers = Object.assign({}, options.headers || {});
    if (options.body && !headers["Content-Type"]) {
      headers["Content-Type"] = "application/json";
    }
    if (options.mutate) {
      Object.assign(headers, STUDIO_HEADER);
    }
    const res = await fetch(path, { ...options, headers });
    const text = await res.text();
    let data = null;
    try {
      data = text ? JSON.parse(text) : null;
    } catch {
      data = { error: text };
    }
    if (!res.ok) {
      const msg = (data && data.error) || res.statusText || "request failed";
      throw new Error(msg);
    }
    return data;
  }

  function setStatus(el, msg, ok) {
    el.textContent = msg || "";
    el.style.color = ok === false ? "var(--fail)" : ok === true ? "var(--ok)" : "var(--muted)";
  }

  async function refreshAgents() {
    const data = await api("/api/agents");
    state.agents = data.agents || [];
    const list = $("agent-list");
    list.innerHTML = "";
    state.agents.forEach((id) => {
      const li = document.createElement("li");
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = id;
      if (id === state.agentId) btn.classList.add("active");
      btn.addEventListener("click", () => selectAgent(id));
      li.appendChild(btn);
      list.appendChild(li);
    });
  }

  async function selectAgent(id) {
    state.agentId = id;
    state.mcpProbe = { servers: [], preferredServer: null };
    state.mcpResult = { raw: null, view: "pretty" };
    state.runOutput = { raw: null, view: state.runOutput?.view || "pretty" };
    state.runEvents = { items: [], view: state.runEvents?.view || "pretty" };
    $("mcp-discover-report").innerHTML = "";
    $("mcp-server").innerHTML = "";
    $("mcp-tool").innerHTML = "";
    $("mcp-tool-desc").textContent = "";
    $("mcp-schema").textContent = "";
    $("mcp-args").value = "{}";
    clearMcpResultView();
    clearRunOutputView();
    renderRunEvents();
    $("btn-mcp-call").disabled = true;
    setStatus($("mcp-discover-status"), "");
    setStatus($("mcp-call-status"), "");
    $("empty-state").classList.add("hidden");
    $("workspace").classList.remove("hidden");
    $("agent-title").textContent = id;
    await refreshAgents();
    await loadAll();
    await refreshOut();
  }

  async function loadAll() {
    const id = state.agentId;
    const [cfg, prompt, rules, skills] = await Promise.all([
      api(`/api/agents/${encodeURIComponent(id)}/config`),
      api(`/api/agents/${encodeURIComponent(id)}/prompt`),
      api(`/api/agents/${encodeURIComponent(id)}/rules`),
      api(`/api/agents/${encodeURIComponent(id)}/skills`),
    ]);
    state.rawConfig = cfg.config;
    fillConfigForm(cfg);
    $("cfg-prompt").value = prompt.content || "";
    $("cfg-rules").value = rules.content || "";
    $("cfg-skills").value = JSON.stringify(skills.skills || [], null, 2);
    await fillSkillPicker(skills.skills || []);
  }

  async function fillSkillPicker(skills) {
    const select = $("cfg-skill-name");
    const previous = state.skillBodyName || select.value;
    select.innerHTML = "";
    const names = skills.map((skill) => skill && skill.name).filter(Boolean);
    if (names.length === 0) {
      $("cfg-skill-body").value = "";
      state.skillBodyName = "";
      return;
    }
    names.forEach((name) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      select.appendChild(option);
    });
    select.value = names.includes(previous) ? previous : names[0];
    await loadSkillBody(select.value);
  }

  async function loadSkillBody(name) {
    if (!name) {
      $("cfg-skill-body").value = "";
      state.skillBodyName = "";
      return;
    }
    const body = await api(
      `/api/agents/${encodeURIComponent(state.agentId)}/skills/${encodeURIComponent(name)}/body`,
    );
    $("cfg-skill-body").value = body.content || "";
    state.skillBodyName = name;
  }

  function fillConfigForm(payload) {
    const c = payload.config;
    const p = c.provider || {};
    const lim = c.limits || {};
    const log = c.logging || {};
    const access = c.access || {};
    $("cfg-base-url").value = p.base_url || "";
    $("cfg-model").value = p.model || "";
    $("cfg-api-key-env").value = p.api_key_env || payload.api_key_env || "";
    $("cfg-api-key-value").value = "";
    $("cfg-clear-api-key").checked = false;
    $("cfg-timeout").value = p.timeout_ms ?? 60000;
    $("cfg-retries").value = p.max_retries ?? 3;
    $("api-key-status").textContent = payload.api_key_env_set
      ? `Secret for ${payload.api_key_env || p.api_key_env} is set.`
      : `Secret for ${payload.api_key_env || p.api_key_env} is not set.`;
    $("cfg-max-iter").value = lim.max_iterations ?? 10;
    $("cfg-max-tokens").value = lim.max_tokens ?? 15000;
    $("cfg-max-duration").value = lim.max_duration_sec ?? 120;
    $("cfg-max-cost").value =
      lim.max_cost == null
        ? ""
        : typeof lim.max_cost === "string"
          ? lim.max_cost
          : `${lim.max_cost.currency || "USD"}:${lim.max_cost.amount}`;
    $("cfg-log-ai").checked = !!log.enable_ai_log;
    $("cfg-log-mcp").checked = !!log.enable_mcp_log;
    $("cfg-log-chat").checked = !!log.enable_chat_history;
    $("cfg-log-dir").value = log.output_dir || "";
    $("cfg-access-mode").value = (access.mode || "strict").toLowerCase();
    $("cfg-builtins").value = joinLines((access.tools && access.tools.builtins) || []);
    $("cfg-home-read").value = joinLines(
      (access.filesystem && access.filesystem.home && access.filesystem.home.read) || []
    );
    $("cfg-home-write").value = joinLines(
      (access.filesystem && access.filesystem.home && access.filesystem.home.write) || []
    );
    $("cfg-run-programs").value = joinLines((access.run && access.run.programs) || []);
    state.mcpServers = normalizeMcpList(c.mcp || []);
    renderMcpList();
    $("cfg-billing").value = JSON.stringify(c.billing || {}, null, 2);
    $("cfg-event-mcp").value = JSON.stringify(c.event_mcp || {}, null, 2);
  }

  function isSetMarker(value) {
    return (
      value &&
      typeof value === "object" &&
      !Array.isArray(value) &&
      value.set === true
    );
  }

  function secretMapToLines(map) {
    return Object.entries(map || {})
      .map(([k, v]) => `${k}=${isSetMarker(v) ? SET_TOKEN : v == null ? "" : String(v)}`)
      .join("\n");
  }

  function linesToSecretMap(text) {
    const out = {};
    for (const line of lines(text)) {
      const i = line.indexOf("=");
      if (i <= 0) continue;
      const key = line.slice(0, i).trim();
      const value = line.slice(i + 1);
      if (!key) continue;
      out[key] = value === SET_TOKEN ? { set: true } : value;
    }
    return out;
  }

  function normalizeMcpList(list) {
    return (list || []).map((s) => {
      const transport =
        s.transport === "http" || (s.url && !s.command) ? "http" : "stdio";
      return {
        name: s.name || "",
        transport,
        command: s.command || "",
        args: Array.isArray(s.args) ? s.args.slice() : [],
        env: s.env && typeof s.env === "object" ? { ...s.env } : {},
        cwd: s.cwd || "",
        url: s.url || "",
        headers: s.headers && typeof s.headers === "object" ? { ...s.headers } : {},
        timeout_ms: s.timeout_ms ?? 20000,
        keep_auth: isSetMarker(s.auth) || (s.auth && typeof s.auth === "object"),
        auth: isSetMarker(s.auth) ? { set: true } : s.auth || null,
      };
    });
  }

  function collectMcpFromDom() {
    const cards = Array.from(document.querySelectorAll("#mcp-list .mcp-card"));
    state.mcpServers = cards.map((card) => {
      const transport = card.querySelector("[data-f='transport']").value;
      const keepAuth = !!card.querySelector("[data-f='keep_auth']")?.checked;
      return {
        name: card.querySelector("[data-f='name']").value.trim(),
        transport,
        command: card.querySelector("[data-f='command']").value.trim(),
        args: lines(card.querySelector("[data-f='args']").value),
        env: linesToSecretMap(card.querySelector("[data-f='env']").value),
        cwd: card.querySelector("[data-f='cwd']").value.trim(),
        url: card.querySelector("[data-f='url']").value.trim(),
        headers: linesToSecretMap(card.querySelector("[data-f='headers']").value),
        timeout_ms: Number(card.querySelector("[data-f='timeout_ms']").value) || 20000,
        keep_auth: keepAuth,
        auth: keepAuth ? { set: true } : null,
      };
    });
  }

  function mcpToWire(server) {
    if (server.transport === "http") {
      const wire = {
        name: server.name,
        transport: "http",
        url: server.url,
        timeout_ms: server.timeout_ms,
      };
      if (server.headers && Object.keys(server.headers).length) {
        wire.headers = server.headers;
      }
      if (server.keep_auth) {
        wire.auth = { set: true };
      }
      return wire;
    }
    const wire = {
      name: server.name,
      transport: "stdio",
      command: server.command,
      timeout_ms: server.timeout_ms,
    };
    if (server.args && server.args.length) wire.args = server.args;
    if (server.env && Object.keys(server.env).length) wire.env = server.env;
    if (server.cwd) wire.cwd = server.cwd;
    return wire;
  }

  function toggleMcpFields(card, isHttp) {
    card.querySelectorAll(".stdio-only").forEach((el) => {
      el.classList.toggle("hidden", isHttp);
    });
    card.querySelectorAll(".http-only").forEach((el) => {
      el.classList.toggle("hidden", !isHttp);
    });
  }

  function renderMcpList() {
    const box = $("mcp-list");
    box.innerHTML = "";
    if (!state.mcpServers.length) {
      const empty = document.createElement("p");
      empty.className = "hint";
      empty.textContent = "No MCP servers yet.";
      box.appendChild(empty);
    }
    state.mcpServers.forEach((s, idx) => {
      const isHttp = s.transport === "http";
      const card = document.createElement("div");
      card.className = "mcp-card";
      card.innerHTML = `
        <header>
          <strong>MCP #${idx + 1}</strong>
          <button type="button" class="btn ghost btn-test-mcp" data-name="${escapeAttr(s.name)}">Test</button>
          <button type="button" class="btn ghost btn-remove-mcp" data-idx="${idx}">Remove</button>
        </header>
        <div class="mcp-grid">
          <label>Name <input data-f="name" type="text" value="${escapeAttr(s.name)}" /></label>
          <label>Transport
            <select data-f="transport">
              <option value="stdio"${!isHttp ? " selected" : ""}>stdio</option>
              <option value="http"${isHttp ? " selected" : ""}>http</option>
            </select>
          </label>
          <label class="stdio-only">Command <input data-f="command" type="text" value="${escapeAttr(s.command)}" /></label>
          <label class="stdio-only">Working dir (cwd) <input data-f="cwd" type="text" value="${escapeAttr(s.cwd || "")}" /></label>
          <label class="stdio-only mcp-span">Args (one per line)
            <textarea data-f="args" rows="3">${escapeHtml((s.args || []).join("\n"))}</textarea>
          </label>
          <label class="stdio-only mcp-span">Env (KEY=value)
            <textarea data-f="env" rows="3">${escapeHtml(secretMapToLines(s.env))}</textarea>
          </label>
          <label class="http-only mcp-span">URL <input data-f="url" type="text" value="${escapeAttr(s.url)}" /></label>
          <label class="http-only mcp-span">Headers (KEY=value)
            <textarea data-f="headers" rows="3">${escapeHtml(secretMapToLines(s.headers))}</textarea>
          </label>
          <label class="http-only row">
            <input data-f="keep_auth" type="checkbox"${s.keep_auth ? " checked" : ""} />
            Keep existing OAuth config
          </label>
          <label>Timeout ms <input data-f="timeout_ms" type="number" min="1" value="${s.timeout_ms ?? 20000}" /></label>
        </div>`;
      box.appendChild(card);
      toggleMcpFields(card, isHttp);
    });

    box.querySelectorAll(".btn-remove-mcp").forEach((btn) => {
      btn.addEventListener("click", () => {
        collectMcpFromDom();
        state.mcpServers.splice(Number(btn.dataset.idx), 1);
        renderMcpList();
      });
    });
    box.querySelectorAll(".btn-test-mcp").forEach((btn) => {
      btn.addEventListener("click", () => {
        const name = (btn.dataset.name || "").trim();
        if (!name) {
          alert("Set MCP server name and Save before testing.");
          return;
        }
        openMcpTest(name);
      });
    });
    box.querySelectorAll("select[data-f='transport']").forEach((sel) => {
      sel.addEventListener("change", () => {
        toggleMcpFields(sel.closest(".mcp-card"), sel.value === "http");
      });
    });
  }

  function escapeAttr(s) {
    return String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/"/g, "&quot;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function buildConfigPayload() {
    const base = JSON.parse(JSON.stringify(state.rawConfig || {}));
    base.provider = base.provider || {};
    base.provider.base_url = $("cfg-base-url").value.trim();
    base.provider.model = $("cfg-model").value.trim();
    base.provider.api_key_env = $("cfg-api-key-env").value.trim();
    base.provider.api_key = null;
    base.provider.timeout_ms = Number($("cfg-timeout").value);
    base.provider.max_retries = Number($("cfg-retries").value);
    base.limits = base.limits || {};
    base.limits.max_iterations = Number($("cfg-max-iter").value);
    base.limits.max_tokens = Number($("cfg-max-tokens").value);
    base.limits.max_duration_sec = Number($("cfg-max-duration").value);
    const cost = $("cfg-max-cost").value.trim();
    if (!cost) {
      base.limits.max_cost = null;
    } else if (cost.includes(":")) {
      const [currency, amount] = cost.split(":");
      base.limits.max_cost = { currency: currency.trim(), amount: amount.trim() };
    } else {
      base.limits.max_cost = { currency: "USD", amount: cost };
    }
    base.logging = base.logging || {};
    base.logging.enable_ai_log = $("cfg-log-ai").checked;
    base.logging.enable_mcp_log = $("cfg-log-mcp").checked;
    base.logging.enable_chat_history = $("cfg-log-chat").checked;
    base.logging.output_dir = $("cfg-log-dir").value.trim() || null;
    base.access = base.access || {};
    base.access.mode = $("cfg-access-mode").value;
    base.access.tools = base.access.tools || {};
    base.access.tools.builtins = lines($("cfg-builtins").value);
    base.access.filesystem = base.access.filesystem || {};
    base.access.filesystem.home = base.access.filesystem.home || {};
    base.access.filesystem.home.read = lines($("cfg-home-read").value);
    base.access.filesystem.home.write = lines($("cfg-home-write").value);
    base.access.run = base.access.run || {};
    base.access.run.programs = lines($("cfg-run-programs").value);
    collectMcpFromDom();
    base.mcp = state.mcpServers.map(mcpToWire);
    base.billing = JSON.parse($("cfg-billing").value || "{}");
    base.event_mcp = JSON.parse($("cfg-event-mcp").value || "{}");
    return base;
  }

  async function saveAll() {
    const id = state.agentId;
    const config = buildConfigPayload();
    const clear = $("cfg-clear-api-key").checked;
    await api(`/api/agents/${encodeURIComponent(id)}/config`, {
      method: "PUT",
      mutate: true,
      body: JSON.stringify({ config, clear_api_key: clear }),
    });
    const secret = $("cfg-api-key-value").value;
    if (secret.trim()) {
      await api(`/api/agents/${encodeURIComponent(id)}/secret`, {
        method: "POST",
        mutate: true,
        body: JSON.stringify({
          key: $("cfg-api-key-env").value.trim(),
          value: secret,
        }),
      });
      $("cfg-api-key-value").value = "";
    }
    await api(`/api/agents/${encodeURIComponent(id)}/prompt`, {
      method: "PUT",
      mutate: true,
      body: JSON.stringify({ content: $("cfg-prompt").value }),
    });
    await api(`/api/agents/${encodeURIComponent(id)}/rules`, {
      method: "PUT",
      mutate: true,
      body: JSON.stringify({ content: $("cfg-rules").value }),
    });
    const skills = JSON.parse($("cfg-skills").value || "[]");
    await api(`/api/agents/${encodeURIComponent(id)}/skills`, {
      method: "PUT",
      mutate: true,
      body: JSON.stringify({ skills }),
    });
    const skillName = $("cfg-skill-name").value;
    if (skillName && skills.some((skill) => skill && skill.name === skillName)) {
      await api(
        `/api/agents/${encodeURIComponent(id)}/skills/${encodeURIComponent(skillName)}/body`,
        {
          method: "PUT",
          mutate: true,
          body: JSON.stringify({ content: $("cfg-skill-body").value }),
        },
      );
    }
    await loadAll();
  }

  function showTab(name) {
    document.querySelectorAll(".tab").forEach((t) => {
      t.classList.toggle("active", t.dataset.tab === name);
    });
    ["config", "mcp", "check", "run"].forEach((tab) => {
      $(`tab-${tab}`).classList.toggle("hidden", tab !== name);
    });
  }

  function flatMcpTools() {
    const out = [];
    (state.mcpProbe.servers || []).forEach((s) => {
      (s.tools || []).forEach((t) => {
        out.push({
          server: s.name,
          name: t.name,
          description: t.description || "",
          input_schema: t.input_schema || { type: "object" },
          error: s.error || null,
        });
      });
    });
    return out;
  }

  function renderMcpDiscoverReport() {
    const root = $("mcp-discover-report");
    root.innerHTML = "";
    (state.mcpProbe.servers || []).forEach((s) => {
      const row = document.createElement("div");
      row.className = "check-item";
      const status = s.error ? "fail" : "ok";
      const detail = s.error
        ? s.error
        : `${(s.tools || []).length} tool(s): ${(s.tools || []).map((t) => t.name).join(", ") || "(none)"}`;
      row.innerHTML = `<span class="badge ${status}">${status}</span><strong>${escapeHtml(
        s.name
      )}</strong><span>${escapeHtml(detail)}</span>`;
      root.appendChild(row);
    });
  }

  function fillMcpServerSelect() {
    const sel = $("mcp-server");
    const prev = sel.value || state.mcpProbe.preferredServer || "";
    const names = (state.mcpProbe.servers || [])
      .filter((s) => !s.error)
      .map((s) => s.name);
    sel.innerHTML = names
      .map((n) => `<option value="${escapeAttr(n)}">${escapeHtml(n)}</option>`)
      .join("");
    if (prev && names.includes(prev)) sel.value = prev;
    else if (names.length) sel.value = names[0];
    fillMcpToolSelect();
  }

  function fillMcpToolSelect() {
    const server = $("mcp-server").value;
    const tools = flatMcpTools().filter((t) => t.server === server);
    const sel = $("mcp-tool");
    const prev = sel.value;
    sel.innerHTML = tools
      .map((t) => `<option value="${escapeAttr(t.name)}">${escapeHtml(t.name)}</option>`)
      .join("");
    if (prev && tools.some((t) => t.name === prev)) sel.value = prev;
    else if (tools.length) sel.value = tools[0].name;
    updateMcpToolMeta();
    $("btn-mcp-call").disabled = !server || !sel.value;
  }

  function schemaStub(schema) {
    const props = (schema && schema.properties) || {};
    const required = new Set((schema && schema.required) || []);
    const stub = {};
    Object.keys(props).forEach((key) => {
      const p = props[key] || {};
      if (Object.prototype.hasOwnProperty.call(p, "default")) {
        stub[key] = p.default;
        return;
      }
      if (!required.has(key)) return;
      switch (p.type) {
        case "string":
          stub[key] = "";
          break;
        case "number":
        case "integer":
          stub[key] = 0;
          break;
        case "boolean":
          stub[key] = false;
          break;
        case "array":
          stub[key] = [];
          break;
        case "object":
          stub[key] = {};
          break;
        default:
          stub[key] = null;
      }
    });
    return stub;
  }

  function updateMcpToolMeta() {
    const server = $("mcp-server").value;
    const toolName = $("mcp-tool").value;
    const tool = flatMcpTools().find((t) => t.server === server && t.name === toolName);
    $("mcp-tool-desc").textContent = tool ? tool.description || "" : "";
    const schema = tool ? tool.input_schema : { type: "object" };
    $("mcp-schema").textContent = JSON.stringify(schema, null, 2);
    $("mcp-args").value = JSON.stringify(schemaStub(schema), null, 2);
  }

  async function discoverMcp(serverFilter) {
    setStatus($("mcp-discover-status"), "Connecting…");
    try {
      const body = serverFilter ? { server: serverFilter } : {};
      const data = await api(`/api/agents/${encodeURIComponent(state.agentId)}/mcp/discover`, {
        method: "POST",
        mutate: true,
        body: JSON.stringify(body),
      });
      state.mcpProbe.servers = data.servers || [];
      if (serverFilter) state.mcpProbe.preferredServer = serverFilter;
      renderMcpDiscoverReport();
      fillMcpServerSelect();
      const failed = (data.servers || []).filter((s) => s.error).length;
      const ok = (data.servers || []).length - failed;
      setStatus(
        $("mcp-discover-status"),
        `Done: ${ok} ok, ${failed} failed`,
        failed === 0
      );
    } catch (e) {
      setStatus($("mcp-discover-status"), e.message, false);
    }
  }

  async function callMcpTool() {
    const server = $("mcp-server").value;
    const tool = $("mcp-tool").value;
    let argumentsObj;
    try {
      argumentsObj = JSON.parse($("mcp-args").value || "{}");
    } catch (e) {
      setStatus($("mcp-call-status"), `Invalid JSON: ${e.message}`, false);
      return;
    }
    setStatus($("mcp-call-status"), "Calling…");
    clearMcpResultView("Calling…");
    try {
      const data = await api(`/api/agents/${encodeURIComponent(state.agentId)}/mcp/call`, {
        method: "POST",
        mutate: true,
        body: JSON.stringify({ server, tool, arguments: argumentsObj }),
      });
      const result = data.result;
      state.mcpResult.raw = result;
      renderMcpResult();
      const nested = mcpResultLooksFailed(result);
      if (nested) {
        setStatus($("mcp-call-status"), nested, false);
      } else {
        setStatus($("mcp-call-status"), "OK", true);
      }
    } catch (e) {
      state.mcpResult.raw = { error: e.message };
      renderMcpResult();
      setStatus($("mcp-call-status"), e.message, false);
    }
  }

  function tryParseJson(text) {
    if (typeof text !== "string") return undefined;
    const trimmed = text.trim();
    if (!trimmed || (trimmed[0] !== "{" && trimmed[0] !== "[" && trimmed[0] !== '"')) {
      return undefined;
    }
    try {
      return JSON.parse(trimmed);
    } catch {
      return undefined;
    }
  }

  function deepParseJsonStrings(value, depth) {
    const d = depth || 0;
    if (d > 8) return value;
    if (typeof value === "string") {
      const parsed = tryParseJson(value);
      if (parsed !== undefined && typeof parsed === "object") {
        return deepParseJsonStrings(parsed, d + 1);
      }
      return value;
    }
    if (Array.isArray(value)) {
      return value.map((item) => deepParseJsonStrings(item, d + 1));
    }
    if (value && typeof value === "object") {
      const out = {};
      Object.keys(value).forEach((key) => {
        out[key] = deepParseJsonStrings(value[key], d + 1);
      });
      return out;
    }
    return value;
  }

  function unwrapMcpPayload(result) {
    if (result == null) return result;
    if (typeof result !== "object") return result;

    if (Array.isArray(result.content) && result.content.length) {
      const texts = result.content
        .filter((c) => c && (c.type === "text" || c.type == null) && typeof c.text === "string")
        .map((c) => c.text);
      if (texts.length === 1) {
        const parsed = tryParseJson(texts[0]);
        return parsed !== undefined ? deepParseJsonStrings(parsed) : texts[0];
      }
      if (texts.length > 1) {
        return texts.map((t) => {
          const parsed = tryParseJson(t);
          return parsed !== undefined ? deepParseJsonStrings(parsed) : t;
        });
      }
    }

    if (result.structuredContent && Object.prototype.hasOwnProperty.call(result.structuredContent, "result")) {
      const inner = result.structuredContent.result;
      if (typeof inner === "string") {
        const parsed = tryParseJson(inner);
        return parsed !== undefined ? deepParseJsonStrings(parsed) : inner;
      }
      return deepParseJsonStrings(inner);
    }

    return deepParseJsonStrings(result);
  }

  function mcpResultLooksFailed(result) {
    if (!result || typeof result !== "object") return null;
    if (result.isError === true) return "Tool reported isError";
    const payload = unwrapMcpPayload(result);
    if (payload && typeof payload === "object" && !Array.isArray(payload) && payload.error) {
      return String(payload.error);
    }
    return null;
  }

  function clearMcpResultView(placeholder) {
    state.mcpResult.raw = null;
    const el = $("mcp-result");
    el.classList.remove("raw-mode");
    el.innerHTML = `<p class="mcp-result-empty">${escapeHtml(placeholder || "")}</p>`;
    $("btn-mcp-copy").disabled = true;
    setMcpViewButtons(state.mcpResult.view);
  }

  function setMcpViewButtons(view) {
    $("btn-mcp-view-pretty").classList.toggle("active", view === "pretty");
    $("btn-mcp-view-raw").classList.toggle("active", view === "raw");
  }

  function renderMcpResult() {
    const el = $("mcp-result");
    const raw = state.mcpResult.raw;
    setMcpViewButtons(state.mcpResult.view);
    $("btn-mcp-copy").disabled = raw == null;
    if (raw == null) {
      el.classList.remove("raw-mode");
      el.innerHTML = "";
      return;
    }
    if (state.mcpResult.view === "raw") {
      el.classList.add("raw-mode");
      el.textContent = JSON.stringify(raw, null, 2);
      return;
    }
    el.classList.remove("raw-mode");
    const payload = unwrapMcpPayload(raw);
    const meta = summarizeMcpPayload(payload);
    el.innerHTML = "";
    if (meta) {
      const p = document.createElement("p");
      p.className = "mcp-result-meta";
      p.textContent = meta;
      el.appendChild(p);
    }
    el.appendChild(renderPrettyValue(payload));
  }

  function summarizeMcpPayload(payload) {
    if (Array.isArray(payload)) {
      return `${payload.length} item${payload.length === 1 ? "" : "s"}`;
    }
    if (payload && typeof payload === "object") {
      return `${Object.keys(payload).length} field${Object.keys(payload).length === 1 ? "" : "s"}`;
    }
    return null;
  }

  function isPlainObject(value) {
    return !!value && typeof value === "object" && !Array.isArray(value);
  }

  function preferredColumnOrder(keys) {
    const preferred = [
      "name",
      "synonym",
      "version",
      "objects_count",
      "slot_status",
      "embedding_status",
      "has_export",
      "in_database",
      "indexed_at",
      "export_path",
      "import_source_path",
    ];
    const set = new Set(keys);
    const ordered = preferred.filter((k) => set.has(k));
    keys.forEach((k) => {
      if (!ordered.includes(k)) ordered.push(k);
    });
    return ordered.slice(0, 10);
  }

  function formatCell(value) {
    if (value === null || value === undefined) return "—";
    if (typeof value === "boolean") return value ? "yes" : "no";
    if (typeof value === "object") return JSON.stringify(value);
    return String(value);
  }

  function renderPrettyValue(value) {
    if (Array.isArray(value) && value.length && value.every(isPlainObject)) {
      return renderObjectTable(value);
    }
    if (typeof value === "string") {
      const pre = document.createElement("pre");
      pre.className = "mcp-json-str";
      pre.style.margin = "0";
      pre.style.whiteSpace = "pre-wrap";
      pre.style.fontFamily = "var(--mono)";
      pre.textContent = value;
      return pre;
    }
    return renderJsonTree(value);
  }

  function renderObjectTable(rows) {
    const wrap = document.createElement("div");
    wrap.className = "mcp-table-wrap";
    const keys = preferredColumnOrder(
      Array.from(
        rows.reduce((acc, row) => {
          Object.keys(row || {}).forEach((k) => acc.add(k));
          return acc;
        }, new Set())
      )
    );
    const table = document.createElement("table");
    table.className = "mcp-table";
    const thead = document.createElement("thead");
    const hr = document.createElement("tr");
    keys.forEach((k) => {
      const th = document.createElement("th");
      th.textContent = k;
      hr.appendChild(th);
    });
    thead.appendChild(hr);
    table.appendChild(thead);
    const tbody = document.createElement("tbody");
    rows.forEach((row) => {
      const tr = document.createElement("tr");
      keys.forEach((k) => {
        const td = document.createElement("td");
        td.textContent = formatCell(row ? row[k] : undefined);
        tr.appendChild(td);
      });
      tbody.appendChild(tr);
    });
    table.appendChild(tbody);
    wrap.appendChild(table);
    return wrap;
  }

  function renderJsonTree(value) {
    if (Array.isArray(value) && value.length && value.every(isPlainObject)) {
      return renderObjectTable(value);
    }
    const root = document.createElement("ul");
    root.className = "mcp-json-tree";
    if (isPlainObject(value)) {
      Object.keys(value).forEach((k) => root.appendChild(renderJsonTreeNode(value[k], k)));
      return root;
    }
    if (Array.isArray(value)) {
      value.forEach((item, idx) => {
        const child = renderJsonTreeNode(item);
        const idxEl = document.createElement("span");
        idxEl.className = "mcp-json-idx";
        idxEl.textContent = `${idx}: `;
        child.insertBefore(idxEl, child.firstChild);
        root.appendChild(child);
      });
      return root;
    }
    root.appendChild(renderJsonTreeNode(value));
    return root;
  }

  function renderJsonTreeNode(value, key) {
    const li = document.createElement("li");
    if (key !== undefined) {
      const keyEl = document.createElement("span");
      keyEl.className = "mcp-json-key";
      keyEl.textContent = key;
      li.appendChild(keyEl);
      li.appendChild(document.createTextNode(": "));
    }

    if (value === null) {
      const span = document.createElement("span");
      span.className = "mcp-json-null";
      span.textContent = "null";
      li.appendChild(span);
    } else if (typeof value === "boolean") {
      const span = document.createElement("span");
      span.className = "mcp-json-bool";
      span.textContent = String(value);
      li.appendChild(span);
    } else if (typeof value === "number") {
      const span = document.createElement("span");
      span.className = "mcp-json-num";
      span.textContent = String(value);
      li.appendChild(span);
    } else if (typeof value === "string") {
      const span = document.createElement("span");
      span.className = "mcp-json-str";
      span.textContent = JSON.stringify(value);
      li.appendChild(span);
    } else if (Array.isArray(value)) {
      if (!value.length) {
        li.appendChild(document.createTextNode("[]"));
      } else if (value.every(isPlainObject)) {
        li.appendChild(document.createTextNode(`[${value.length}]`));
        const nested = document.createElement("div");
        nested.style.marginTop = "0.35rem";
        nested.appendChild(renderObjectTable(value));
        li.appendChild(nested);
      } else {
        li.appendChild(document.createTextNode(`[${value.length}]`));
        const ul = document.createElement("ul");
        value.forEach((item, idx) => {
          const child = renderJsonTreeNode(item);
          const idxEl = document.createElement("span");
          idxEl.className = "mcp-json-idx";
          idxEl.textContent = `${idx}: `;
          child.insertBefore(idxEl, child.firstChild);
          ul.appendChild(child);
        });
        li.appendChild(ul);
      }
    } else if (isPlainObject(value)) {
      const keys = Object.keys(value);
      if (!keys.length) {
        li.appendChild(document.createTextNode("{}"));
      } else {
        li.appendChild(document.createTextNode(`{${keys.length}}`));
        const ul = document.createElement("ul");
        keys.forEach((k) => ul.appendChild(renderJsonTreeNode(value[k], k)));
        li.appendChild(ul);
      }
    } else {
      li.appendChild(document.createTextNode(String(value)));
    }
    return li;
  }

  async function copyMcpResult() {
    const raw = state.mcpResult.raw;
    if (raw == null) return;
    const payload =
      state.mcpResult.view === "raw" ? raw : unwrapMcpPayload(raw);
    const text = typeof payload === "string" ? payload : JSON.stringify(payload, null, 2);
    try {
      await navigator.clipboard.writeText(text);
      setStatus($("mcp-call-status"), "Copied", true);
    } catch (e) {
      setStatus($("mcp-call-status"), `Copy failed: ${e.message}`, false);
    }
  }

  function openMcpTest(serverName) {
    state.mcpProbe.preferredServer = serverName || null;
    showTab("mcp");
    discoverMcp(serverName || undefined);
  }

  async function runCheck() {
    const report = await api(`/api/agents/${encodeURIComponent(state.agentId)}/check`, {
      method: "POST",
      mutate: true,
    });
    const root = $("check-report");
    root.innerHTML = "";
    (report.items || []).forEach((item) => {
      const row = document.createElement("div");
      row.className = "check-item";
      row.innerHTML = `<span class="badge ${item.status}">${item.status}</span><strong>${escapeHtml(
        item.name
      )}</strong><span>${escapeHtml(item.detail)}</span>`;
      root.appendChild(row);
    });
    const inv = report.inventory || {};
    const invEl = document.createElement("div");
    invEl.className = "panel";
    invEl.innerHTML = `<h2>Inventory</h2>
      <p class="hint">Available tools: ${(inv.tools_available || []).map(escapeHtml).join(", ") || "(none)"}</p>
      <p class="hint">MCP connected: ${(inv.mcp_connected || []).map((s) => s.name).join(", ") || "(none)"}</p>
      <p class="hint">MCP failed: ${(inv.mcp_failed || []).map((p) => p[0]).join(", ") || "(none)"}</p>
      <p class="hint">Gated: ${(inv.mcp_gated || []).map(escapeHtml).join(", ") || "(none)"}</p>`;
    root.appendChild(invEl);
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function closeEvents() {
    if (state.eventSource) {
      state.eventSource.close();
      state.eventSource = null;
    }
  }

  function clearRunOutputView(placeholder) {
    state.runOutput.raw = null;
    const el = $("run-output");
    el.classList.remove("raw-mode");
    el.innerHTML = `<p class="mcp-result-empty">${escapeHtml(placeholder || "")}</p>`;
    $("btn-run-out-copy").disabled = true;
    setRunOutViewButtons(state.runOutput.view);
  }

  function setRunOutViewButtons(view) {
    $("btn-run-out-pretty").classList.toggle("active", view === "pretty");
    $("btn-run-out-raw").classList.toggle("active", view === "raw");
  }

  function setRunEventsViewButtons(view) {
    $("btn-run-events-pretty").classList.toggle("active", view === "pretty");
    $("btn-run-events-raw").classList.toggle("active", view === "raw");
  }

  function renderRunOutput() {
    const el = $("run-output");
    const raw = state.runOutput.raw;
    setRunOutViewButtons(state.runOutput.view);
    $("btn-run-out-copy").disabled = raw == null;
    if (raw == null) {
      el.classList.remove("raw-mode");
      el.innerHTML = "";
      return;
    }
    if (typeof raw === "string") {
      el.classList.remove("raw-mode");
      el.innerHTML = "";
      const pre = document.createElement("pre");
      pre.style.margin = "0";
      pre.style.whiteSpace = "pre-wrap";
      pre.style.fontFamily = "var(--mono)";
      pre.textContent = raw;
      el.appendChild(pre);
      return;
    }
    if (state.runOutput.view === "raw") {
      el.classList.add("raw-mode");
      el.textContent = JSON.stringify(raw, null, 2);
      return;
    }
    el.classList.remove("raw-mode");
    const payload = deepParseJsonStrings(raw);
    const meta = summarizeMcpPayload(payload);
    el.innerHTML = "";
    if (meta) {
      const p = document.createElement("p");
      p.className = "mcp-result-meta";
      p.textContent = meta;
      el.appendChild(p);
    }
    el.appendChild(renderPrettyValue(payload));
  }

  function renderRunEvents() {
    const el = $("run-events");
    const items = state.runEvents.items || [];
    setRunEventsViewButtons(state.runEvents.view);
    if (!items.length) {
      el.classList.remove("raw-mode");
      el.innerHTML = "";
      return;
    }
    if (state.runEvents.view === "raw") {
      el.classList.add("raw-mode");
      el.textContent = items.map((ev) => JSON.stringify(ev)).join("\n");
      el.scrollTop = el.scrollHeight;
      return;
    }
    el.classList.remove("raw-mode");
    el.innerHTML = "";
    const meta = document.createElement("p");
    meta.className = "mcp-result-meta";
    meta.textContent = `${items.length} event${items.length === 1 ? "" : "s"}`;
    el.appendChild(meta);
    items.forEach((ev, idx) => {
      const block = document.createElement("div");
      block.className = "run-event-block";
      const head = document.createElement("div");
      head.className = "run-event-head";
      const typ = ev && ev.type != null ? String(ev.type) : "event";
      head.textContent = `#${idx + 1} ${typ}`;
      block.appendChild(head);
      block.appendChild(renderPrettyValue(deepParseJsonStrings(ev)));
      el.appendChild(block);
    });
    el.scrollTop = el.scrollHeight;
  }

  async function copyRunOutput() {
    const raw = state.runOutput.raw;
    if (raw == null) return;
    const payload =
      state.runOutput.view === "raw" || typeof raw === "string"
        ? raw
        : deepParseJsonStrings(raw);
    const text = typeof payload === "string" ? payload : JSON.stringify(payload, null, 2);
    try {
      await navigator.clipboard.writeText(text);
      setStatus($("run-status"), "Copied", true);
    } catch (e) {
      setStatus($("run-status"), `Copy failed: ${e.message}`, false);
    }
  }

  async function startRun() {
    closeEvents();
    state.runEvents.items = [];
    renderRunEvents();
    clearRunOutputView();
    $("run-log").textContent = "";
    const body = {
      prompt: $("run-prompt").value,
      files: lines($("run-files").value),
      home: $("run-home").value.trim() || null,
      max_iterations: numOrNull($("run-max-iter").value),
      max_tokens: numOrNull($("run-max-tokens").value),
      max_duration_sec: numOrNull($("run-max-duration").value),
      max_cost: $("run-max-cost").value.trim() || null,
      save_chat_history: $("run-save-chat").checked,
    };
    const res = await api(`/api/agents/${encodeURIComponent(state.agentId)}/runs`, {
      method: "POST",
      mutate: true,
      body: JSON.stringify(body),
    });
    state.currentRunId = res.run_id;
    $("btn-cancel").disabled = false;
    setStatus($("run-status"), `Running ${res.run_id}`, true);
    const es = new EventSource(`/api/runs/${encodeURIComponent(res.run_id)}/events`);
    state.eventSource = es;
    es.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        appendEvent(data);
        if (data.type === "done") {
          es.close();
          state.eventSource = null;
          $("btn-cancel").disabled = true;
          pollResult(res.run_id);
        }
      } catch (err) {
        appendEvent({ type: "message", text: String(err) });
      }
    };
    es.onerror = () => {
      // Browser retries; also poll status.
      pollResult(res.run_id);
    };
  }

  function numOrNull(v) {
    const t = String(v || "").trim();
    if (!t) return null;
    const n = Number(t);
    return Number.isFinite(n) ? n : null;
  }

  function appendEvent(data) {
    state.runEvents.items.push(data);
    renderRunEvents();
  }

  async function pollResult(runId) {
    try {
      const status = await api(`/api/runs/${encodeURIComponent(runId)}`);
      setStatus($("run-status"), `${status.status} (${runId})`, status.status !== "failed");
      if (status.output) {
        state.runOutput.raw = status.output;
        renderRunOutput();
      } else if (status.error) {
        state.runOutput.raw = String(status.error);
        renderRunOutput();
      }
      if (status.status !== "running") {
        $("btn-cancel").disabled = true;
        await refreshOut();
      }
    } catch (err) {
      setStatus($("run-status"), err.message, false);
    }
  }

  async function cancelRun() {
    if (!state.currentRunId) return;
    await api(`/api/runs/${encodeURIComponent(state.currentRunId)}/cancel`, {
      method: "POST",
      mutate: true,
    });
    setStatus($("run-status"), "Cancel requested", true);
  }

  async function loadLog(which) {
    if (!state.currentRunId) return;
    const data = await api(
      `/api/runs/${encodeURIComponent(state.currentRunId)}/logs?which=${encodeURIComponent(which)}`
    );
    $("run-log").textContent = data.content || "";
  }

  async function refreshOut() {
    if (!state.agentId) return;
    const data = await api(`/api/agents/${encodeURIComponent(state.agentId)}/out`);
    const list = $("out-list");
    list.innerHTML = "";
    (data.files || []).forEach((f) => {
      const li = document.createElement("li");
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = `${f.name} (${f.size} B)`;
      btn.addEventListener("click", async () => {
        const preview = await api(
          `/api/agents/${encodeURIComponent(state.agentId)}/out/${encodeURIComponent(f.name)}`
        );
        $("out-preview").textContent = preview.content || "";
      });
      li.appendChild(btn);
      list.appendChild(li);
    });
  }

  function bind() {
    $("btn-refresh").addEventListener("click", () =>
      refreshAgents().catch((e) => alert(e.message))
    );
    $("btn-new").addEventListener("click", async () => {
      const id = prompt("New agent id");
      if (!id) return;
      try {
        await api("/api/agents", {
          method: "POST",
          mutate: true,
          body: JSON.stringify({ id }),
        });
        await refreshAgents();
        await selectAgent(id);
      } catch (e) {
        alert(e.message);
      }
    });
    $("btn-import").addEventListener("click", async () => {
      const id = prompt("Agent id to import into");
      if (!id) return;
      const from = prompt("Path to template/bundle on this machine");
      if (!from) return;
      try {
        await api("/api/agents/import", {
          method: "POST",
          mutate: true,
          body: JSON.stringify({ id, from, force: true }),
        });
        await refreshAgents();
        await selectAgent(id);
      } catch (e) {
        alert(e.message);
      }
    });
    document.querySelectorAll(".tab").forEach((tab) => {
      tab.addEventListener("click", () => showTab(tab.dataset.tab));
    });
    $("btn-add-mcp").addEventListener("click", () => {
      collectMcpFromDom();
      state.mcpServers.push({
        name: "",
        transport: "stdio",
        command: "",
        args: [],
        env: {},
        cwd: "",
        url: "",
        headers: {},
        timeout_ms: 20000,
        keep_auth: false,
        auth: null,
      });
      renderMcpList();
    });
    $("cfg-skill-name").addEventListener("change", () =>
      loadSkillBody($("cfg-skill-name").value).catch((err) =>
        setStatus($("save-status"), err.message || String(err), true),
      ),
    );
    $("btn-save").addEventListener("click", async () => {
      try {
        await saveAll();
        setStatus($("save-status"), "Saved", true);
      } catch (e) {
        setStatus($("save-status"), e.message, false);
      }
    });
    $("btn-check").addEventListener("click", async () => {
      try {
        setStatus($("check-status"), "Running…");
        await runCheck();
        setStatus($("check-status"), "Done", true);
      } catch (e) {
        setStatus($("check-status"), e.message, false);
      }
    });
    $("btn-mcp-discover").addEventListener("click", () => discoverMcp());
    $("mcp-server").addEventListener("change", () => fillMcpToolSelect());
    $("mcp-tool").addEventListener("change", () => updateMcpToolMeta());
    $("btn-mcp-call").addEventListener("click", () => callMcpTool());
    $("btn-mcp-view-pretty").addEventListener("click", () => {
      state.mcpResult.view = "pretty";
      renderMcpResult();
    });
    $("btn-mcp-view-raw").addEventListener("click", () => {
      state.mcpResult.view = "raw";
      renderMcpResult();
    });
    $("btn-mcp-copy").addEventListener("click", () => copyMcpResult());
    $("btn-run-out-pretty").addEventListener("click", () => {
      state.runOutput.view = "pretty";
      renderRunOutput();
    });
    $("btn-run-out-raw").addEventListener("click", () => {
      state.runOutput.view = "raw";
      renderRunOutput();
    });
    $("btn-run-out-copy").addEventListener("click", () => copyRunOutput());
    $("btn-run-events-pretty").addEventListener("click", () => {
      state.runEvents.view = "pretty";
      renderRunEvents();
    });
    $("btn-run-events-raw").addEventListener("click", () => {
      state.runEvents.view = "raw";
      renderRunEvents();
    });
    $("btn-run-events-clear").addEventListener("click", () => {
      state.runEvents.items = [];
      renderRunEvents();
    });
    $("btn-run").addEventListener("click", async () => {
      try {
        await startRun();
      } catch (e) {
        setStatus($("run-status"), e.message, false);
      }
    });
    $("btn-cancel").addEventListener("click", () =>
      cancelRun().catch((e) => setStatus($("run-status"), e.message, false))
    );
    document.querySelectorAll("[data-log]").forEach((btn) => {
      btn.addEventListener("click", () =>
        loadLog(btn.dataset.log).catch((e) => {
          $("run-log").textContent = e.message;
        })
      );
    });
  }

  bind();
  refreshAgents().catch((e) => {
    $("empty-state").textContent = `Failed to load agents: ${e.message}`;
  });
})();
