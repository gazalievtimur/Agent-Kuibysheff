# CLI agent contract

This repository provides a stateless CLI worker. Repository discovery, run
coordination, review, and application of generated changes belong to an
external orchestrator.

## Invocation

```text
agent_Kuibyshev \
  --config <FILE> \
  --settings-dir <DIR> \
  --prompt <TEXT> \
  --home <DIR> \
  [--files <PATH>...] \
  [--max-iterations N] \
  [--max-tokens N] \
  [--max-duration-sec N]
```

- `--config` contains provider, MCP, limits, logging, and optional `access`
  policy configuration.
- `--settings-dir` contains `master_prompt.md`, `skills.dsl`, and optional
  `rules.md`.
- `--prompt` is the task for one run.
- `--files` are UTF-8 files embedded into the model context as read-only
  inputs. They are not copied into home. Each file is truncated to 50,000
  characters in the context. When `access` is present, each file must fall
  under `access.filesystem.input_roots`.
- `--home` is the root for built-in `home.*` filesystem tools. The agent
  creates it when necessary.

The process prints exactly one JSON `RunOutput` document to stdout. A run-level
failure is represented by `stop_reason: "error"` and an error message in
`result`. Policy denials and sandbox unavailability are returned as tool-result
errors (for example `PolicyDenied`, `SandboxUnavailable`) without performing
the side effect.

## Access policy (fail-closed)

`access` in the config file is optional:

| Mode | Behavior |
|------|----------|
| `access` omitted (legacy) | `home.list` / `home.read` / `home.write` and `local_tools.*` keep prior home/workspace semantics. `home.run` is **not** advertised. |
| `access` present (strict) | Only listed built-ins, path grants, and program aliases are allowed. Everything else is denied. |

Effective built-in tools:

```text
builtins = access.tools.builtins ∩ skills.allowed_tools ∩ advertised built-ins
mcp      = every server.tool discovered from configured mcp entries
effective = builtins ∪ mcp
```

- Tool names must be qualified `server.tool` (bare names are rejected).
- Declared MCP servers are trusted capabilities: their `command` / `env` and the
  tools from `tools/list` are allowed without listing them in skills.
- There is no generic `network` config section. The agent process may call only
  the configured `provider.base_url` origin (no proxy, no cross-origin
  redirects). MCP processes may use the network. `home.run` payloads never get
  network access.

See [`agent-config.example.yaml`](agent-config.example.yaml) for the schema and
comments.

### Migration

1. Keep running without `access` if you only need filesystem tools (legacy).
2. To enable `home.run`, add an `access` block that includes `home.run` in
   `tools.builtins`, path grants under `filesystem.home`, and at least one
   entry in `run.programs` (`name` is the model-facing alias; `executable` is a
   host path resolved against the config file directory).
3. Update `skills.dsl` `allowed_tools` to qualified names that intersect the
   built-in allowlist.
4. If you use `--files` under strict mode, declare `filesystem.input_roots`.

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
Under legacy mode the CLI permits access anywhere below home, but never
outside it. Under strict `access`, only the configured home read/write
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
{"server":"home","tool":"read","arguments":{"path":"in/src/foo.rs","max_chars":50000}}
{"server":"home","tool":"write","arguments":{"path":"out/src/foo.rs","content":"..."}}
{"server":"home","tool":"run","arguments":{"program":"python","args":["solution.py"],"timeout_ms":30000}}
```

Paths for `list` / `read` / `write` must be relative, cannot contain `..`, and
are checked after filesystem canonicalization. Symlinks that resolve outside
home (or outside a grant) are rejected.

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
  effect rule. By default they live under `~/.agent-kuibyshev/logs` on the host
  running the agent, not inside `--home`.
- `RunOutput.logs.system_log` points to the append-only tracing file when
  logging is initialized.
- `RunOutput.logs.chat_log` points to the saved transcript when chat history
  logging is enabled via config or `--save-chat-history`.
