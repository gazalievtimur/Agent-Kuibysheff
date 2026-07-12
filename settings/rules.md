# Rust coding guidelines

When generating or refactoring Rust code under `out/`, follow idiomatic Rust
practices from the bundled [rust-skills](https://github.com/leonardomso/rust-skills)
knowledge base at `.cursor/skills/rust-skills/`.

Apply these priorities without loading every rule into context:

- Prefer borrowing (`&T`, `&str`) over cloning; accept `&[T]` and `&str`, not `&Vec<T>` / `&String`.
- Use `Result` and `?` for fallible code; avoid `.unwrap()` / `.expect()` in library paths.
- Use `thiserror` for library errors and `anyhow` for application boundaries when appropriate.
- Keep `unsafe` minimal, documented with `// SAFETY:`, and justified.
- Match project lint policy: `clippy::all` + `clippy::pedantic` with `-D warnings`.

For detailed guidance, use the index in `.cursor/skills/rust-skills/SKILL.md` and read
only the relevant files from `.cursor/skills/rust-skills/rules/` (prefixes: `own-`,
`err-`, `mem-`, `async-`, `anti-`, `perf-`, `test-`, `lint-`).

# Workspace and artifact rules

- Never attempt to modify attached input files or any path outside home.
- Use `in/` as orchestrator-provided source material.
- Use `home.run` to execute programs under home when the task requires running code.
- For a coding task, write complete replacement files under `out/` using paths
  relative to the target repository.
- Put unified diffs under `patches/` only when patch output is requested.
- Put non-applicable reports or reasoning under `notes/`.
- Before completing a coding task, write `out/manifest.json` with
  `schema_version`, `summary`, `files_written`, `patches`, and `apply_mode`.
- `files_written` contains paths relative to `out/`. `patches` contains paths
  relative to home.
- `apply_mode` must be one of `copy_out`, `patches`, or `none`.
- Never claim that generated files were applied, committed, pushed, or tested
  unless a supplied tool result explicitly proves it.

# Response protocol

- Output JSON only. Never mix JSON with other text.
- Never simulate tool calls in plain text.
- Use `done=false` while files still need to be written.
- Example first turn:

```json
{
  "done": false,
  "thought": "Writing summary file",
  "tool_calls": [
    {
      "server": "home",
      "tool": "write",
      "arguments": {
        "path": "out/summary.md",
        "content": "# Summary\n\n- item"
      }
    }
  ],
  "result": null
}
```

- Example final turn:

```json
{
  "done": true,
  "thought": "All required files were written",
  "tool_calls": [],
  "result": "Created out/summary.md and out/manifest.json"
}
```
