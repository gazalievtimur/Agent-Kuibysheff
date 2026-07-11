# Prompt examples for agent_Kuibyshev

Use these as `--prompt` values when calling the CLI or when configuring an
external orchestrator.

Every example assumes:
- `--settings-dir ./settings`
- `--home <sandbox>`
- JSON-only replies from the model
- file writes through `home.write` and command execution through `home.run`
  with `server: "home"`

## General template

```text
<Describe the task clearly>

Required steps:
1. First response: done=false, one home.write call creating out/<file1>
2. Second response: done=false, one home.write call creating out/<file2>
3. ...
N. Final response: done=true with a short result

Return JSON only on every turn.
```

Rules:
- Keep one main deliverable per `home.write` call when possible.
- Always include a final `out/manifest.json` for coding or file-generation tasks.
- Do not set `done=true` until required files were written.

## README summary (demo)

```text
Summarize the attached README in 5-8 bullet points.

Required steps:
1. First response: done=false, one home.write call creating out/summary.md
2. Second response: done=false, one home.write call creating out/manifest.json
3. Third response: done=true with a short result

Return JSON only on every turn.
```

PowerShell:

```powershell
cargo run -- `
  --config .\agent-config.local-demo.yaml `
  --settings-dir .\settings `
  --prompt "Summarize the attached README in 5-8 bullet points.`n`nRequired steps:`n1. First response: done=false, one home.write call creating out/summary.md`n2. Second response: done=false, one home.write call creating out/manifest.json`n3. Third response: done=true with a short result`n`nReturn JSON only on every turn." `
  --home .\demo-home `
  --files .\README.md
```

Or:

```powershell
.\run-demo.ps1
```

## Code change from snapshot in home/in

Use when the orchestrator prepared source files under `home/in/`.

```text
Implement the requested change using files under in/ as the source snapshot.

Required steps:
1. First response: done=false, read relevant files from in/ with home.read if needed
2. Next responses: done=false, write changed files under out/ with home.write
3. Before finishing: done=false, one home.write call creating out/manifest.json
4. Final response: done=true with a short result

Return JSON only on every turn.
Do not modify attached files or paths outside home.
Write complete replacement files under out/ using repository-relative paths.
```

## Review-only task

Use when the orchestrator only needs analysis, not repository changes.

```text
Review the attached files and identify risks, missing tests, and suggested fixes.

Required steps:
1. First response: done=false, one home.write call creating out/review.md
2. Second response: done=false, one home.write call creating out/manifest.json with apply_mode=none
3. Final response: done=true with a short result

Return JSON only on every turn.
Do not claim that fixes were applied.
```

## Multi-file feature

```text
Add feature <name> based on the attached specification.

Required steps:
1. First response: done=false, write out/src/<module>.rs
2. Second response: done=false, write out/tests/<module>_test.rs
3. Third response: done=false, write out/manifest.json
4. Final response: done=true with a short result

Return JSON only on every turn.
Use one home.write call per file.
```

## Expected JSON shapes

First turn:

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

Final turn:

```json
{
  "done": true,
  "thought": "All required files were written",
  "tool_calls": [],
  "result": "Created out/summary.md and out/manifest.json"
}
```

## Common mistakes to avoid in prompts

- Do not ask the model to "just write the answer" without mentioning `home.write`.
- Do not omit the JSON-only requirement.
- Do not combine many file writes and `done=true` in the same turn unless the
  task is trivial and the model is reliable.
- Do not assume attached `--files` are writable; they are read-only context.

See also:
- [CONTRACT.md](../CONTRACT.md)
- [settings/master_prompt.md](../settings/master_prompt.md)
- [settings/rules.md](../settings/rules.md)
