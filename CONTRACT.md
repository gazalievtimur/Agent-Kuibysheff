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

- `--config` contains provider, MCP, limits, and logging configuration.
- `--settings-dir` contains `master_prompt.md`, `skills.dsl`, and optional
  `rules.md`.
- `--prompt` is the task for one run.
- `--files` are UTF-8 files embedded into the model context as read-only
  inputs. They are not copied into home. Each file is truncated to 50,000
  characters in the context.
- `--home` is the only directory available to the built-in filesystem tools.
  The agent creates it when necessary.

The process prints exactly one JSON `RunOutput` document to stdout. A run-level
failure is represented by `stop_reason: "error"` and an error message in
`result`.

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
The CLI sandbox permits access anywhere below home, but never outside it.

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
```

Paths must be relative, cannot contain `..`, and are checked after filesystem
canonicalization. Symlinks that resolve outside home are rejected.

Tool access is also restricted by `skills.dsl`. Qualified names such as
`home.read` and `home.write` are recommended.

## Security boundary

- The orchestrator must not pass a writable target repository as `--home`.
- Input files are read for context only and cannot be changed by built-in
  tools.
- The CLI never runs `git apply`, copies output to a repository, creates a
  commit, or opens a pull request.
- MCP servers are explicitly configured capabilities. Their permissions are
  outside the home filesystem sandbox and must be reviewed by the
  orchestrator/operator.
- Configured logging paths are an explicit exception to the home-only side
  effect rule.
