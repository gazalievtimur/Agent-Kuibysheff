# AoC bank example schema

Copy this directory to `../aoc-bank/` (gitignored) and replace samples with real tasks.

Each JSON file is one task:

| Field | Required | Description |
| --- | --- | --- |
| `id` | yes | Stable identifier (also used as filename stem), e.g. `2024-01-1` |
| `url` | no | Advent of Code or other task URL |
| `title` | no | Short title |
| `text` | yes | Full problem statement shown to the agent via MCP |
| `input` | yes | Puzzle input returned by `aoc_get_input` |
| `expected` | yes | Ground-truth answer for the harness only — never exposed by MCP |

Filename may be `{id}.json` or any `*.json`; the `id` field inside the file is authoritative.
