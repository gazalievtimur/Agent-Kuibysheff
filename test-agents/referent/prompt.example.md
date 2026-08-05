# Referent prompt examples

## Jira issue key

```text
Собери первичную информацию по задаче PROJ-123.

Required steps:
1. Fetch the Jira issue and linked Confluence pages through atlassian MCP tools.
2. Describe screenshots and diagrams if the model supports vision.
3. Write out/task_brief.md with summary, requirements, docs, images, and open questions.
4. Optionally write out/sources.json and out/images/*.md for detailed image notes.
5. Write out/manifest.json with apply_mode=none.
6. Final response: done=true with a short result.

Return JSON only on every turn.
```

PowerShell:

```powershell
cargo run --bin agent_Kuibysheff -- run `
  --config .\test-agents\referent\agent-config.example.yaml `
  --settings-dir .\test-agents\referent `
  --prompt "Собери первичную информацию по задаче PROJ-123.`n`nRequired steps:`n1. Fetch the Jira issue and linked Confluence pages through atlassian MCP tools.`n2. Describe screenshots and diagrams if the model supports vision.`n3. Write out/task_brief.md with summary, requirements, docs, images, and open questions.`n4. Optionally write out/sources.json and out/images/*.md for detailed image notes.`n5. Write out/manifest.json with apply_mode=none.`n6. Final response: done=true with a short result.`n`nReturn JSON only on every turn." `
  --home .\demo-home\referent
```

## Jira URL in prompt

```text
Подготовь brief по https://your-company.atlassian.net/browse/PROJ-456

Follow the same required steps as above. Extract the issue key from the URL.
Return JSON only on every turn.
```

## Confluence page only

```text
Собери первичную информацию из Confluence page id 123456789.

Required steps:
1. Load the page with confluence_get_page.
2. Fetch child pages or linked issues only when clearly relevant.
3. Write out/task_brief.md and out/manifest.json.
4. Final response: done=true.

Return JSON only on every turn.
```

## Advent of Code task id

```text
Solve AoC task 2024-01-1.

Work one turn at a time: each reply must be exactly one JSON object (never multiple).
Do not pre-emit future turns. Steps across turns:
1. Fetch the task statement with aoc_get_task and the input with aoc_get_input.
2. Write a Python solution under home with home.write.
3. Run it with home.run (program=python). Debug using stdout/stderr until correct.
4. Final response: done=true with result equal to only the final answer string.

Return JSON only on every turn.
```

PowerShell (or use `scripts/aoc-eval.ps1`):

```powershell
$env:AOC_BANK_DIR = (Resolve-Path .\local\aoc-bank).Path
cargo run --release --bin agent_Kuibysheff -- run `
  --config .\test-agents\referent\agent-config.aoc.example.yaml `
  --settings-dir .\test-agents\referent `
  --prompt "Solve AoC task 2024-01-1.`n`nWork one turn at a time: each reply must be exactly one JSON object (never multiple). Do not pre-emit future turns.`nSteps across turns:`n1. Fetch the task statement with aoc_get_task and the input with aoc_get_input.`n2. Write a Python solution under home with home.write.`n3. Run it with home.run (program=python). Debug using stdout/stderr until correct.`n4. Final response: done=true with result equal to only the final answer string.`n`nReturn JSON only on every turn." `
  --home .\local\aoc-runs\manual\2024-01-1
```

## Expected artifacts

### Research

```text
out/
  task_brief.md       primary human-readable brief
  sources.json        optional structured source list
  images/             optional per-image descriptions
  manifest.json       required completion marker
```

### AoC solve

```text
solution.py           working solver (typical)
input.txt             optional persisted puzzle input
# Evaluation uses RunOutput.result only (not out/ files)
```
