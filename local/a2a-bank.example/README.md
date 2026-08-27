# A2A live bank samples

Opt-in live regression for `kbshff a2a`: Agent Card discovery, optional Bearer
gate, and a real `SendMessage` → `run_agent_prompt` roundtrip.

Copy to a working bank (gitignored):

```powershell
Copy-Item -Recurse .\local\a2a-bank.example .\local\a2a-bank
```

```bash
cp -R ./local/a2a-bank.example ./local/a2a-bank
```

Then run `scripts/a2a-regression.ps1` / `.sh`.

## Tasks

| File | Kind | LLM | Checks |
| --- | --- | --- | --- |
| `card-smoke-01.json` | `card` | no | `GET /.well-known/agent-card.json` (name, interfaces) |
| `bearer-gate-01.json` | `bearer` | no | RPC without token → 401; card stays public |
| `send-smoke-01.json` | `send` | yes | `SendMessage`, task completed, home file + result token |

Each task uses an isolated project under `local/a2a-runs/<run-id>/` and home
`homes/<task-id>/`.

## Requirements

- Built release binary: `cargo build --release --bin kbshff`
- `test-agents/a2a-probe/` imported per run
- Provider API key for `send` tasks (`api_key_env` from config)
- Optional: set `A2A_LIVE_TOKEN` for bearer tasks (harness sets a default when unset)

Manual smoke (no harness):

```text
kbshff init a2a-probe --project-root ./local/a2a-smoke --force
kbshff config --project-root ./local/a2a-smoke --agent a2a-probe import --from ./test-agents/a2a-probe --force
kbshff a2a --project-root ./local/a2a-smoke --agent a2a-probe --bind 127.0.0.1:8787
curl -s http://127.0.0.1:8787/.well-known/agent-card.json
```

With [a2acli](https://github.com/a2aproject/a2a-rs):

```text
a2acli --base-url http://127.0.0.1:8787 send "Create out/a2a-smoke.txt with exactly one line: A2A_SMOKE_OK. When finished, set result to A2A_OK."
```
