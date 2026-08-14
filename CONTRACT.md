# CLI agent contract

This repository provides a stateless CLI worker. Repository discovery, run
coordination, review, and application of generated changes belong to an
external orchestrator.

Multi-stage product workflows may chain several `run` invocations (different
`--agent` / `--home` per stage under one `--project-root`) and hand off `out/`
artifacts between them. An example is the 1C conveyor driven by
[`scripts/1c-dev-run.ps1`](scripts/1c-dev-run.ps1) with stage profiles under
[`test-agents/1c-*`](test-agents/). The same four profiles can be driven from an
IDE as **four ACP registrations** plus an external prepare/promote helper
([`scripts/1c-dev-acp-prepare.ps1`](scripts/1c-dev-acp-prepare.ps1)); see
[`extensions/vscode/README.md`](extensions/vscode/README.md).
The full 1C copy-unit (prompts, adapters, product YAML) is a local-only package
under `workflows/1c-dev/` (gitignored; restore from git history for testing).

## Invocation

```text
kbshff run \
  --project-root <DIR> \
  --agent <ID> \
  --prompt <TEXT> \
  [--home <REL_UNDER_KUIBYSHEFF>] \
  [--files <PATH>...] \
  [--run-id ID] \
  [--max-iterations N] \
  [--max-tokens N] \
  [--max-duration-sec N] \
  [--max-cost CURRENCY:AMOUNT]
```

- `--project-root` is the product/workspace directory that owns `.kuibysheff/`.
- `--agent` selects the protected profile
  `{project-root}/.kuibysheff/protected/agents/<id>/` (settings + `agent-config.yaml`).
- Operators do **not** pass storage paths for config/settings; the agent owns the layout.
- `--prompt` is the task for one run.
- `--files` are UTF-8 files embedded into the model context as read-only
  inputs. They are not copied into home. Each file is truncated to 50,000
  characters in the context. Under strict `access`, each file must fall
  under `access.filesystem.input_roots`. Paths under `.kuibysheff/protected/`
  are always denied.
- `--home` is optional and must be **relative** under `.kuibysheff/` (never under
  `protected/`). Default: `homes/<agent>`. Absolute `--home` is rejected.
- External settings enter the profile only via `config import --from <PATH>`
  (write into protected store, then validate; see `config import` below).

`limits.*` stop the run (iterations / cumulative token budget / wall clock /
optional exact-decimal monetary budget).
Wall-clock expiry cancels in-flight provider/MCP waits cooperatively and is
reported as `stop_reason: "limit_reached"` (same as other limit hits). Tool
side effects that already started may still finish.
`provider.history` controls how much chat context is sent to the model on each
step: after a fixed prefix (system prompt + initial user message), only the
newest `max_tail_messages` turns are kept, and the whole window is capped at
`max_chars` UTF-8 characters. Defaults are `30` / `200000`. These knobs belong
with the model because larger context windows need a larger working history;
they are independent of `limits.max_tokens`.
`billing` resolves each physical provider attempt through configured
provider-reported, MCP, and catalog sources. `RunOutput.usage.cost` is always
present; missing prices are `partial`/`unavailable`, never zero. `max_cost` is
fail-soft when any attempt is unpriced and can overshoot by one completed
provider request. The full contract is in [docs/BILLING.md](docs/BILLING.md).

The `run` process prints exactly one JSON `RunOutput` document to stdout. A
run-level failure is represented by `stop_reason: "error"` and an error message
in `result`. In that case the process also exits with a non-zero status
(stdout still contains the JSON). `goal_reached` and `limit_reached` exit 0.
Serialize failure of `RunOutput` likewise prints a minimal error JSON and exits
non-zero. Policy denials and sandbox unavailability are returned as tool-result
errors (for example `PolicyDenied`, `SandboxUnavailable`) without performing
the side effect.
`RunOutput.run_id` is generated per prompt unless `--run-id` supplies an
orchestrator identity. Per-attempt provider request IDs, usage, source,
precision, and amount are nested under `usage.cost.requests`.

### Management commands

Commands other than `run` (for example `init`, `check`, `acp`, `config`) print
human-readable text and use process exit codes — except `acp`, which speaks
JSON-RPC on stdio (see below). They do **not** emit `RunOutput` JSON.
`config` accepts `--format text|json` (default `text`).

```text
agent_Kuibysheff help
agent_Kuibysheff help init
agent_Kuibysheff help check
agent_Kuibysheff help acp
agent_Kuibysheff help config
agent_Kuibysheff --help

kbshff init <agent-id> --project-root <DIR> [--force] [-i|--interactive]

kbshff check --project-root <DIR> --agent <ID> \
  [--skip-provider] [--skip-mcp] [--skip-sandbox]

kbshff acp \
  --agent <ID> \
  [--project-root <DIR>] \
  [--home <REL>] \
  [--max-iterations N] \
  [--max-tokens N] \
  [--max-duration-sec N] \
  [--max-cost CURRENCY:AMOUNT] \
  [--save-chat-history]

kbshff config --project-root <DIR> --agent <ID> [--format text|json] \
  import --from <PATH> [--force]
  | show | provider … | limits … | access … | mcp … | billing … \
  | event-mcp … | skill … | prompt … | rules … | tools effective [--connect]
```

`init` creates the protected profile under
`.kuibysheff/protected/agents/<id>/` (`master_prompt.md`, `skills.dsl`,
`rules.md`, `agent-config.yaml`). With `--interactive`, the CLI prompts for
`provider` and `limits`.

`config import` copies an external config file or settings directory into the
protected profile. External paths are treated as untrusted payloads until
contents are installed under `.kuibysheff/protected/` and validated there
(`load_config` + safety + skills parse). `--force` overwrites a non-empty
profile. If the profile has no `agent-config.yaml` yet, the agent bootstraps a
minimal fail-closed `access` policy first so settings can be imported;
external `access` is imported when present and becomes authoritative only after
that write-then-validate step. Directory import may omit a config file
(settings-only); missing `access` in an external config is filled with the same
minimal policy before validation.

`check` probes the protected profile without running the agent loop.

### Protected store trust boundary

Settings under `.kuibysheff/protected/` are readable/writable only by the
`agent_Kuibysheff` process (load + `config` CRUD). Tool paths (`home.*`,
`local_tools.*`, `--files`) hard-deny that tree. `home.run` sandbox specs must
not grant it. Stdio MCP children use `mcp-runtime/{agent}/{server}/` with a
cleared environment and require a working OS sandbox probe unless
`KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1` (dev/emergency only; also required for the
nested Docker Desktop Linux SWE-bench runner, which lacks `clone3` —
`scripts/swebench-regression-linux-docker.*`).

Exit code `0` for `check` only when every probe is `ok` or intentionally `skip`.

### ACP (IDE, messengers, mail bridges)

`acp` starts an [Agent Client Protocol](https://agentclientprotocol.com/)
agent on **stdio**. This is the shared integration boundary for IDE hosts and
for external applications (messengers, email, bots) that speak ACP over pipes.
Kuibysheff does **not** embed Telegram/email/Slack clients or credentials; those
stay in the external bridge process.

IDE layering (VS Code):

```text
VS Code UI  ←AHP→  VS Code Agent Host  ←ACP→  kbshff acp
```

External bridge layering:

```text
Messenger/Mail API  ←→  Bridge process  ←ACP stdio pipes→  kbshff acp
```

#### Stream contract

| Stream | Content |
|--------|---------|
| **stdin** | ACP JSON-RPC requests/notifications from the host or bridge only |
| **stdout** | ACP JSON-RPC responses/notifications only (no `RunOutput`, no logs) |
| **stderr** | Diagnostics (`tracing`, startup errors). Must be drained on a separate pipe so a full stderr buffer cannot stall the child |

Each completed ACP prompt emits a final agent message with token count and
known cost (or `unavailable`) because ACP v1 `PromptResponse` has no standard
usage field.

Rules for bridge authors:

- Spawn with three separate pipes (do not merge stdout and stderr).
- Keep **one long-lived process per agent** (`--project-root` / `--agent` /
  optional `--home`); do not restart for every chat message.
- On stdin EOF the agent exits; restart the process if the bridge needs another
  session after exit.
- Each `session/prompt` is **stateless** relative to prior turns: the engine does
  not keep chat history across prompts. The bridge must attach any thread/context
  it needs inside the prompt text (or via `--files` / home `in/` prepared outside).
- Messenger/mail tokens, webhooks, and user identity mapping belong only in the
  bridge — never in Kuibysheff config.

Minimal spawn sketch (pseudo-Rust):

```rust
let mut child = Command::new("agent_Kuibysheff")
    .args(["acp", "--project-root", project, "--agent", agent_id])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped()) // drain concurrently
    .spawn()?;
// Speak ACP over child.stdin / child.stdout (e.g. agent-client-protocol::ByteStreams).
```

#### Protocol notes

- Kuibysheff does **not** implement an AHP host; VS Code already owns that.
- The official AHP Rust crates (`ahp` / `ahp-ws`) are **clients** to a host and
  are not used here.
- Protocol version: ACP schema **v1** via `agent-client-protocol` 2.x
  (`InitializeRequest` / `session/new` / `session/prompt` / `session/cancel` /
  `session/update`).
- Each `session/prompt` runs the same worker wiring as `run` (access, sandbox,
  MCP, `AgentEngine`). Deliverables still land under `--home`.
- `session/new` supplies `cwd`. Effective project root is non-empty session
  `cwd`, else CLI `--project-root`. Relative config/settings/home paths resolve
  under `{project-root}/.kuibysheff/` the same way as `run`.
- Fail-closed `access` is unchanged; policy denials surface as tool errors.
- `init_tracing` is idempotent for the same log directory inside one process so
  sequential prompts do not panic; switching log directories mid-process is rejected.

Example VS Code ACP Client settings when the **product folder** is the
workspace (see [`extensions/vscode/README.md`](extensions/vscode/README.md)):

```json
"acp.agents": {
  "1c-analyst": {
    "command": "agent_Kuibysheff",
    "args": [
      "acp",
      "--project-root", "${workspaceFolder}",
      "--agent", "1c-analyst"
    ]
  }
}
```

Profiles live under `.kuibysheff/protected/agents/<id>/` after `init` or
`config import`. Session `cwd` from the IDE is preferred as project root when
non-empty.

`run` remains the one-shot orchestrator contract (single `RunOutput` JSON on
stdout). Multi-agent IDE or bridge flows register one ACP process per profile;
stage handoff, chat thread mapping, and the plan gate stay outside the Rust
binary.

## Access policy (fail-closed)

**Breaking (0.2.0):** `access` in the config file is **required**. Omitting it
fails config validation. Prior releases treated omission as silent legacy
(permissive FS); that footgun is removed.

| Mode | Behavior |
|------|----------|
| `access.mode: legacy` | Explicit opt-in: `home.list` / `home.read` / `home.write` and `local_tools.*` keep permissive home/workspace/input semantics. `home.run` is **not** advertised. Must not set `tools` / `filesystem` / `run` alongside. |
| `access` present (strict; default when `mode` omitted) | Only listed built-ins, path grants, and program aliases are allowed. Everything else is denied. |

Effective tools are computed as follows:

```text
KNOWN_BUILTINS = {home.list, home.read, home.write, home.run, local_tools.search_docs, local_tools.read_file}

effective_builtins = KNOWN_BUILTINS ∩ access.tools.builtins ∩ skills.allowed_tools
effective_mcp      = all_discovered_mcp_tools ∩ skills.allowed_tools
effective_tools    = effective_builtins ∪ effective_mcp
```

- Tool names must be qualified `server.tool` (bare names are rejected).
- Built-ins are gated by the intersection of the config allowlist and the
  skills allowlist. Both must grant the tool for it to be available.
- MCP tools are gated by the skills allowlist. Every tool advertised by a
  configured MCP server in `tools/list` is allowed only if it is also listed in
  `skills.allowed_tools`. The operator must still review each configured MCP
  server's command, environment, URL, headers, and OAuth settings before enabling
  it.
- `rules.md` and the `policy` string inside a `skills.dsl` block are prompt-only
  guidance for the model. They are not enforced by the runtime policy engine.
- There is no generic `network` config section. The agent process may call only
  the configured `provider.base_url` origin (no proxy, no cross-origin
  redirects). MCP stdio subprocesses and configured MCP HTTP endpoints may use
  the network. `home.run` payloads never get network access.

Example config and skills:

```yaml
# agent-config.yaml
access:
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
      - home.run
      - local_tools.search_docs
      - local_tools.read_file
  filesystem:
    home:
      read: ["."]
      write: ["out/"]
    workspace:
      root: "."
      read: ["."]
  run:
    programs:
      - name: python
        executable: /usr/bin/python3
```

```text
# settings/skills.dsl
skill "coding" {
  policy: "Use home.* for workspace writes. Use local_tools.* for research. Use MCP tools when listed below."
  allowed_tools: [
    "home.list",
    "home.read",
    "home.write",
    "home.run",
    "local_tools.search_docs",
    "local_tools.read_file",
    "mcp_docs.search"
  ]
}
```

See [`agent-config.example.yaml`](agent-config.example.yaml) for the full schema
and comments.

### Migration

1. Add an `access` block (required). For a temporary permissive FS opt-in use
   `access: { mode: legacy }`. Prefer strict grants in production.
2. To enable `home.run`, use strict mode with `home.run` in `tools.builtins`,
   path grants under `filesystem.home`, and at least one entry in
   `run.programs` (`name` is the model-facing alias; `executable` is a host
   path resolved against the config file directory).
3. Update `skills.dsl` `allowed_tools` to qualified names that intersect the
   built-in allowlist.
4. If you use `--files` under strict mode, declare `filesystem.input_roots`.
5. If you use MCP servers, add each MCP tool name to `skills.dsl`
   `allowed_tools` as well; otherwise it is denied at runtime.

## Home workspace

The orchestrator may prepare this layout before invocation:

```text
home/
  in/              read-only snapshot prepared by the orchestrator
  out/             generated files, using target-repository-relative paths
    manifest.json
  patches/         optional unified diff files
  notes/           optional material that must not be applied
```

The layout is a convention between coding-agent settings and the orchestrator.
Under `access.mode: legacy` the CLI permits access anywhere below home, but
never outside it. Under strict `access`, only the configured home read/write
prefixes are available.

For a successful coding task, the agent must produce `out/manifest.json`:

```json
{
  "schema_version": 1,
  "summary": "Short description of the result",
  "files_written": ["src/foo.rs"],
  "patches": [],
  "apply_mode": "copy_out"
}
```

Fields:

- `schema_version`: currently `1`.
- `summary`: concise description for review.
- `files_written`: paths under `out/`, expressed relative to `out/`.
- `patches`: paths relative to home, normally under `patches/`.
- `apply_mode`: `copy_out`, `patches`, or `none`. This is a recommendation,
  never an instruction executed by the CLI.

The orchestrator must validate the manifest and generated paths, create a diff,
run its review policy and tests, and explicitly apply accepted changes to the
target repository.

## Built-in tools

Tool calls use `server: "home"`:

```json
{"server":"home","tool":"list","arguments":{"path":"."}}
{"server":"home","tool":"read","arguments":{"path":"in/src/foo.rs","offset":0,"max_chars":50000}}
{"server":"home","tool":"write","arguments":{"path":"out/src/foo.rs","content":"..."}}
{"server":"home","tool":"run","arguments":{"program":"python","args":["solution.py"],"timeout_ms":30000}}
```

Paths for `list` / `read` / `write` must be relative, cannot contain `..`, and
are checked after filesystem canonicalization. Symlinks that resolve outside
home (or outside a grant) are rejected.

`home.read` and `local_tools.read_file` return a UTF-8 **character window**:
`content`, `offset`, `chars_returned`, `truncated`, and `next_offset`. There is
**no whole-file size reject** — large files are read gradually by passing
`next_offset` from a truncated response as the next `offset`. Per-call
`max_chars` is capped (200000) as a context-window guard only.
`local_tools.search_docs` has no silent file-count / depth / per-file byte
caps; only the explicit `max_results` argument limits how many hits are
returned (default 8, max 100).

`home.run` executes a **policy alias** (`program`) mapped to a pre-resolved
host executable. Arguments are a raw argv vector (no shell, no `PATH`
lookup). The working directory is the home root. Default timeout is 30
seconds; the configured maximum is `access.run.max_timeout_ms` (default
120000). The tool returns `stdout`, `stderr`, truncation flags, `exit_code`,
and `timed_out`.

Tool access is also restricted by `skills.dsl` for built-ins. Qualified names
such as `home.read`, `home.write`, and `home.run` are required.

## OS sandbox for `home.run`

Payloads never run via a plain process spawn. The CLI uses a platform backend:

| Platform | Mechanism | Guarantees |
|----------|-----------|------------|
| Linux | unprivileged user/mount/pid/ipc/net namespaces, pivot_root, caps drop, seccomp denylist | No host network; filesystem limited to explicit binds; process tree killed on timeout (`pidfd`) |
| Windows | AppContainer (empty capabilities) + Job Object | No network capabilities / loopback exemption; ACL grants only for configured paths; job kill on timeout |

Prerequisites:

- Linux: user namespaces enabled; on Ubuntu,
  `kernel.apparmor_restrict_unprivileged_userns` may need to be `0` or the
  helper binary needs an AppArmor profile that allows `userns`. See
  [`crates/sandbox-linux/TESTING.md`](crates/sandbox-linux/TESTING.md).
- Windows: AppContainer APIs available (typical desktop/server Windows).

If the host cannot enforce the sandbox, `home.run` fails closed
(`SandboxUnavailable`) and does not start the payload. The orchestrator does
**not** need to wrap the agent in a container for `home.run` isolation; an
outer container remains optional defense-in-depth.

## Security boundary

- The orchestrator must not pass a writable target repository as `--home`.
- Input files are read for context only and cannot be changed by built-in
  tools.
- The CLI never runs `git apply`, copies output to a repository, creates a
  commit, or opens a pull request.
- Provider HTTP is limited to the configured origin; MCP and `home.run` follow
  the trust model above.
- MCP servers are explicitly configured capabilities. Their permissions are
  outside the home filesystem sandbox and must be reviewed by the
  orchestrator/operator.
- Configured logging paths are an explicit exception to the home-only side
  effect rule. By default they live under `~/.agent-kuibysheff/logs` on the host
  running the agent, not inside `--home`.
- `RunOutput.logs.system_log` points to the append-only tracing file when
  logging is initialized.
- `RunOutput.logs.chat_log` points to the saved transcript when chat history
  logging is enabled via config or `--save-chat-history`.
