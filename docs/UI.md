# Local agent studio (`kbshff-studio`)

**Handoff doc for humans and coding agents working on the GUI.**  
Paste or `@`-mention this file when the task is about the browser studio, not about orchestrators (`run` / `acp` / `a2a`).

Operator console for configuring and test-running agents in a browser.
It is a separate binary from `kbshff`. The studio process does not link the
agent library: every mutation, probe, and test run is a `kbshff` subprocess.
It is **not** part of the orchestrator contract in [CONTRACT.md](../CONTRACT.md):
orchestrators keep using `run` / `acp` / `a2a`. The studio only edits the same
protected profile files that `kbshff config` and `kbshff check` already use.

---

## Scope (what “GUI work” means)

| In scope | Out of scope |
| --- | --- |
| `kbshff-studio` HTTP server, `ui/studio/`, operator CLI it calls | Banks / 1C stage pipelines |
| Agent profile CRUD via studio API | Changing CONTRACT.md orchestrator surface |
| Configure forms, secrets, MCP probe, check, single test run | Multi-agent orchestration UI |
| Docs/tests for studio | Replacing CLI `config` / `check` / `run` |

Primary code owners:

| Path | Role |
| --- | --- |
| `src/cli/ui.rs` | `kbshff ui` forwards `--project-root`, `--bind`, `--open` to `kbshff-studio` |
| `crates/kbshff-studio/` | Axum server, subprocess client, single test run + SSE |
| `src/commands/config/redact.rs` | `config document` secret redaction / merge |
| `src/commands/mcp_probe.rs` | `kbshff mcp discover` / `mcp call` |
| `ui/studio/index.html` | Shell, tabs, forms |
| `ui/studio/app.js` | Client API, MCP pretty result, run SSE |
| `ui/studio/studio.css` | Layout / theme |
| `docs/UI.md` | This document |

Supporting MCP pieces used by the probe (not studio-only):

- `src/mcp/stdio_client/registry.rs` — `list_tool_descriptors`, `call_discovered_tool`
- Stdio children run under MCP isolation (cleared env + allowlisted keys from config; OS sandbox unless `KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1`)

---

## Start / rebuild

```text
cargo build --release --bin kbshff --bin kbshff-studio
kbshff-studio
```

A release build places `kbshff-studio` next to `kbshff`. With no flags the studio
uses the current directory as `--project-root` and opens `http://127.0.0.1:8790/`.
`--no-open` leaves the browser closed. `--kbshff <PATH>` overrides the CLI lookup.

`kbshff ui` does the same when `kbshff-studio` sits next to the `kbshff` binary
(as it does under `target/debug` or `target/release`), on `PATH`, or in `KBSHFF_STUDIO`.
The studio finds `kbshff` via `--kbshff`, then `KBSHFF_BIN`, then a sibling executable, then `PATH`.

Windows example (1C agent home used in local ops):

```text
kbshff-studio --project-root "C:\Agents Kbshff\1C"
```

- Default bind: `127.0.0.1:8790` (A2A defaults to `8787`).
- Only **loopback** addresses are accepted.
- The browser opens by default (`cmd /C start` on Windows, `xdg-open` on Linux). `--no-open` skips that. `--open` is accepted and does the same as the default.
- Stop with Ctrl+C.
- Open `http://127.0.0.1:8790/`.

### Debug vs release assets (important)

`rust_embed` in `kbshff-studio` on `ui/studio/`:

- **Debug builds** often serve HTML/JS/CSS from the **filesystem** (`ui/studio/`). UI tweaks can appear after a hard refresh without rebuilding.
- **API / Rust routes** always come from the **compiled studio binary**. After adding/changing endpoints or CLI flags the studio calls, rebuild and **restart** `kbshff-studio`.
- Symptom of stale binary + fresh UI: new tab visible, `POST .../mcp/discover` returns HTTP **404 Not Found**.

Release / install builds embed assets at compile time — rebuild the studio binary for both UI and API. `kbshff` itself does not embed the UI.

---

## UI map (tabs)

Sidebar: agent list, Refresh, New agent, Import.

| Tab | Purpose |
| --- | --- |
| **Configure** | Provider, limits, logging, **MCP form cards** (not raw JSON), access, billing, event-mcp, prompt / rules / skills textareas. Save writes profile files. |
| **MCP** | Discover tools from **saved** config; pick server/tool; JSON args (stubbed from `inputSchema`); Call; Pretty/Raw/Copy result. |
| **Check** | Same probes as `kbshff check`; inventory (tools, MCP connected/failed/gated). |
| **Test run** | One active run per studio process: prompt, files, home, limits; SSE events; `RunOutput`; log tails; `homes/<id>/out/` preview. |

MCP card **Test** jumps to the MCP tab and discovers that server.

### Configure → MCP forms

Each server card: name, transport `stdio` | `http`, command/args/cwd/env or url/headers, timeout.  
Secrets in env/headers are shown as set-markers after GET; PUT with `{"set": true}` keeps stored values.

### MCP probe UX

- Uses **saved** `agent-config.yaml` (Save Configure first).
- Discover connects **one server at a time** so one failure does not hide others.
- Pretty view unwraps MCP `content[].text` JSON (and nested stringified JSON) and renders arrays-of-objects as a table.
- Raw view shows the full MCP envelope.
- Green “OK” means the HTTP/studio call succeeded; nested tool payloads may still contain `"error"` — Pretty status turns red when unwrapped payload has `error` or `isError: true`.

### Secrets

- Set API key via Configure → writes profile `.env` for `provider.api_key_env`.
- Value is never returned by GET config.

---

## Security

- Mutations require header `X-Kbshff-Studio: 1` (browser client sets it for mutate calls).
- Config GET redacts `provider.api_key` and MCP env/headers/OAuth secrets → `{"set": true}`.
- PUT with that marker keeps the stored value. `null` api_key does **not** clear an existing inline key unless `clear_api_key` is true.
- Out-file previews: single path segment under agent home `out/` only (no `..` escape).

---

## HTTP API

Prefix `/api`. Static UI: `/`, `/studio.css`, `/app.js` (and embed folder).

Mutating routes need `X-Kbshff-Studio: 1`. Errors: `{ "error": "..." }` with appropriate status.

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/api/agents` | `{ agents: [id, ...] }` |
| POST | `/api/agents` | `{ id, force? }` → `init` |
| POST | `/api/agents/import` | `{ id, from, force? }` |
| GET/PUT | `/api/agents/:id/config` | Redacted GET; PUT merges secrets |
| GET/PUT | `/api/agents/:id/prompt` | `{ content }` |
| GET/PUT | `/api/agents/:id/rules` | `{ content }` |
| GET/PUT | `/api/agents/:id/skills` | `{ skills: [...] }` |
| POST | `/api/agents/:id/secret` | Write env var for API key |
| POST | `/api/agents/:id/check` | Check report + inventory |
| POST | `/api/agents/:id/mcp/discover` | Body optional `{ "server": "name" }` → `{ servers: [{ name, tools, error? }] }` tools include `name`, `description?`, `input_schema` |
| POST | `/api/agents/:id/mcp/call` | `{ server, tool, arguments }` → `{ server, tool, result }` |
| POST | `/api/agents/:id/runs` | Start single run |
| GET | `/api/runs/:id` | Run status / output |
| GET | `/api/runs/:id/events` | SSE |
| POST | `/api/runs/:id/cancel` | Cancel |
| GET | `/api/runs/:id/logs?which=` | Log tail (`ai` / `mcp` / …) |
| GET | `/api/agents/:id/out` | File list |
| GET | `/api/agents/:id/out/:name` | Preview (size-capped) |

---

## Data layout (on disk)

Under `--project-root`:

```text
.kuibysheff/protected/agents/<id>/
  agent-config.yaml
  master_prompt.md
  rules.md
  skills.dsl
  .env                 # secrets (api keys); never returned by GET
.kuibysheff/homes/<id>/
  in/ out/             # created on run when grants allow; out/ previewed in Test run
  logs/                # studio sets AGENT_LOG_DIR here for Test run
```

Studio never invents a second config format — same files as CLI. Test run sets
`AGENT_LOG_DIR` to `{home}/logs` (default `homes/<agent>/logs`, or the Home field)
so AI/MCP/trace files land next to the task, not under `~/.agent-kuibysheff/logs`.

---

## Operational pitfalls (GUI / MCP)

1. **Stale `kbshff` on PATH** — use `target/debug/kbshff` (or release) built from this branch.
2. **404 on new API** — UI from disk, routes from old binary → rebuild + restart.
3. **MCP discover/call empty or fail** — config not saved; wrong `--project-root`; stdio sandbox; missing command.
4. **conf_doc HTTP 503 via MCP while curl works** — Windows system HTTP proxy (e.g. `127.0.0.1:10809`) hijacks httpx. Fix in conf-doc MCP client: `trust_env=False`; agent env may set `NO_PROXY=127.0.0.1,localhost`.
5. **MCP result “duplicated”** — FastMCP structured + text; conf-doc tools use `structured_output=False`. Studio Pretty unwraps `content` so operators see one payload.
6. **One test run at a time** — second start while running returns conflict from `JobManager`.
7. **code-index without daemon** — do **not** use stock `bsl-indexer serve` for kbshff agents (that mode requires a live `daemon` + `CODE_INDEX_HOME` and returns `daemon_offline`). Prefer:

   ```yaml
   command: C:\Python312\python.exe
   args: [C:\mcp\code-index\offline_serve.py, --path, "cf=C:\\path\\to\\cf"]
   ```

   `offline_serve.py` is stdio MCP over local `<cf>/.code-index/index.db` + CLI (`query`, `grep-body`, …). No daemon process. Build/refresh DB with `bsl-indexer index <cf>`. Tool calls pass `"repo":"cf"`.

   Also: `cf=C:\repo` args must keep the alias — kbshff `resolve_stdio_arg` must not join the whole token under `mcp-runtime/` (fixed).

---

## Tests

```text
cargo test -p kbshff-studio
cargo test --lib commands::config::redact
cargo test --lib commands::mcp_probe
cargo test --lib commands::agent
```

Studio tests use a stub `kbshff` (list, config envelope, missing studio header → 403, second run → 409, cancel). Redaction, empty MCP discover, and `out/` path escape are covered on the CLI.

UI has no automated browser tests; exercise manually with `kbshff-studio --open`.

---

## Extending the GUI (conventions)

1. **New read-only page data** — GET under `/api/agents/:id/...`, no studio header required (match existing GETs).
2. **New mutation / side effect** (including MCP connect/call) — require `X-Kbshff-Studio: 1`; client: `api(path, { mutate: true, ... })`.
3. **Static UI** — edit `ui/studio/*`; keep JS vanilla (no bundler). In debug, refresh; for release, rebuild binary.
4. **Secrets** — never echo values in GET; use redaction markers; document in this file.
5. **Do not** couple studio to banks/1C stages or expand beyond single-run operator tooling without an explicit product decision.
6. New studio behavior belongs in a `kbshff` command. The studio crate only spawns that command and maps JSON.

---

## Related docs

| Doc | When |
| --- | --- |
| [CONTRACT.md](../CONTRACT.md) | Orchestrator / CLI stability — studio is explicitly outside |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Security boundaries, protected store |
| [LOGGING.md](LOGGING.md) | Log file names used by Test run tails |
| [OUTPUT.md](OUTPUT.md) | `RunOutput` shape shown after a run |
| [EVENT_MCP.md](EVENT_MCP.md) | event-mcp config block on Configure tab |

---

## Quick checklist for a new chat

```text
Task: agent studio GUI (`kbshff-studio`)
Doc: docs/UI.md
Code: crates/kbshff-studio/*, ui/studio/*, operator commands in src/commands/
Build: cargo build --bin kbshff --bin kbshff-studio
Run: kbshff-studio --project-root <DIR>
Test: cargo test -p kbshff-studio
Remember: debug UI may hot-read files; API needs restart after Rust changes
```
