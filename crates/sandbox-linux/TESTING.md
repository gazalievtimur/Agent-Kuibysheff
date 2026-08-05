# Linux sandbox testing notes

Integration tests in `tests/namespaces.rs` need a real Linux host with
unprivileged user namespaces. They are `cfg(target_os = "linux")` and do not
run on Windows.

Miri CI runs only library unit tests (`cargo miri test -p sandbox-linux --lib`).
Namespace integration tests need host `stat`/`unshare`/`fork` and are not
Miri-isolatable.

## Known lab host

| Item | Value |
|------|--------|
| Address | `192.168.68.119` |
| SSH alias | `ubuntu-laptop` (see `~/.ssh/config`) |
| User | `aidev` |
| Identity | `~/.ssh/id_ed25519` |
| Kernel (observed) | `7.0.0-27-generic` |
| Rust | rustup, `1.85.0` at `$HOME/.cargo/bin` |

```sshconfig
Host ubuntu-laptop
    HostName 192.168.68.119
    User aidev
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 60
    ServerAliveCountMax 3
```

Smoke check:

```bash
ssh ubuntu-laptop 'uname -r; rustc --version; cat /proc/sys/kernel/unprivileged_userns_clone; cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns'
```

Expect `unprivileged_userns_clone=1`. For mounts inside a userns to work, either
`apparmor_restrict_unprivileged_userns=0` (dev only) or an AppArmor profile that
allows `userns` for the helper/test binary (preferred long-term).

## AppArmor / userns (important)

Ubuntu’s `kernel.apparmor_restrict_unprivileged_userns=1` lets processes create a
user namespace but denies `CAP_SYS_ADMIN` mounts unless the binary is allowed.
Symptoms: `mount MS_PRIVATE` or `proc` → `EPERM`, or uid map write failures.

Temporary (reverts on reboot unless persisted):

```bash
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
```

Do **not** treat a permanent global `=0` as the product design. The sandbox
supervisor is the trusted creator of namespaces; payloads stay tightly confined.
Target model: AppArmor profile with `userns` for the agent helper only (same
idea as `/etc/apparmor.d/bwrap-userns-restrict`).

Passwordless sudo is not configured on the lab host; the sysctl must be set
interactively on the machine.

## Sync from Windows (dev machine)

Repo path contains a space (`Agent Kuibysheff`). Prefer extracting to a path
**without** spaces on the Linux host.

PowerShell (from the Windows checkout):

```powershell
$dest = "$env:TEMP\agent-kuibysheff-linux.tgz"
tar -czf $dest --exclude=target --exclude=.git --exclude=src/.code-index --exclude=.env `
  -C "C:\Git" "Agent Kuibysheff"
scp -o BatchMode=yes $dest ubuntu-laptop:/tmp/agent-kuibysheff-linux.tgz
ssh -o BatchMode=yes ubuntu-laptop @"
export PATH=`$HOME/.cargo/bin:`$PATH
rm -rf `$HOME/src/agent-kuibysheff-test
tar -xzf /tmp/agent-kuibysheff-linux.tgz -C /tmp
mv '/tmp/Agent Kuibysheff' `$HOME/src/agent-kuibysheff-test
cd `$HOME/src/agent-kuibysheff-test
cargo test -p sandbox-linux --test namespaces -- --nocapture --test-threads=1
"@
```

Notes:

- Use a **new directory name** each sync if a previous `target/` was created by
  Docker as root (`Permission denied` on `rm -rf`). Or `sudo rm -rf` that tree.
- Quote carefully in PowerShell; `$HOME` must be escaped as `` `$HOME `` inside
  double-quoted SSH commands.
- Nested userns inside Docker is a poor substitute for host tests (uid map /
  `proc` often fail even with `--privileged`).

## Test command (on Linux)

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd ~/src/agent-kuibysheff-test   # or current sync dir
cargo test -p sandbox-linux --test namespaces -- --nocapture --test-threads=1
```

## AoC agent regression (on Linux)

Same live-agent gate as Windows `scripts/aoc-regression.ps1`, but as bash:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd ~/src/agent-kuibysheff-test
chmod +x ./scripts/*.sh
cp -Rn ./local/aoc-bank.example ./local/aoc-bank   # if bank missing
# Provide API key via .env / agent-config.local.yaml / POLZA_API_KEY
./scripts/aoc-regression.sh
./scripts/aoc-regression.sh --task-id 2024-01-1
./scripts/check.sh --skip-aoc   # fmt/clippy/cargo test only
```

Requires Node.js + `python3` on `PATH`, a populated `local/aoc-bank/`, and working
unprivileged userns (see AppArmor notes above). Details:
[local/README.md](../../local/README.md).

Cross-check from Windows without running tests:

```powershell
cargo check -p sandbox-linux --target x86_64-unknown-linux-gnu
```

## What the namespace tests cover

| Test | Intent |
|------|--------|
| `probe_succeeds_or_explains` | Probe unshare + uid map + `MS_PRIVATE` (or skip with clear unavailable) |
| `echo_under_grants` | Script under grants; stdout contains marker; exit 0 |
| `deny_sibling_write` | Sibling of write grant not writable; setup must not exit 70 |
| `timeout_kills_tree` | Short deadline → `timed_out` or exit `124` |
| `network_namespace_has_no_foreign_ifaces` | Empty netns: only `lo` under `/sys/class/net` |
| `argv_metacharacters_are_literal` | Shell metacharacters stay literal argv (not executed) |
| `truncates_large_stdout` | Large stdout sets truncation flag and respects `max_output_chars` |

Agent / HomeFs E2E (Windows CI / local): `tests/integration.rs::model_can_home_run_via_native_sandbox`
uses `sandbox_e2e_fixture` through `HomeFs` + `PolicyToolExecutor` + native backend.

Grants passed into OS sandboxes must be **absolute** paths under `--home`
(`sandbox::absolute_home_grants`); relative prefixes alone are insufficient.

## Implementation pitfalls already fixed

Keep these in mind when debugging regressions:

1. **Helper re-exec** — `LinuxSandbox::run` re-execs `current_exe` with
   `AGENT_KUIBYSHEFF_LINUX_SANDBOX_HELPER`. Test binaries must enter helper mode
   via `.init_array` → `try_run_helper()` in `lib.rs` (otherwise the harness
   recurses / hangs).
2. **Uid map failure** — kill the clone child via pidfd before releasing the
   start barrier (`helper.rs`), or PID1 continues without caps and misreports
   mount errors.
3. **Write grant + remount RO** — do not `MS_RDONLY` remount paths that are
   also writable; recursive remount of binds under `/tmp` can `EPERM`. Prefer
   `mount_setattr` / non-recursive remount (`mount.rs`).
4. **`proc` mount** — mount onto `new_root/proc` **before** `pivot_root`, as
   PID 1 in the new pid namespace.
5. **Shebang scripts** — `fexecve` cannot run `#!` scripts (`ENOENT`); after
   pivot use `execve` on the absolute path (`pid1.rs`).
6. **Merged `/bin` → `/usr/bin`** — bind using the **logical** guest path
   (keep `/bin`), canonicalize only the mount source, or shebang `#!/bin/sh`
   breaks.
7. **`LinuxSandbox::run` Ok ≠ success** — helper exit 70 (pid1 setup failure)
   still returns `Ok(SandboxLaunchResult)`. Assert `exit_code` / stderr in
   tests.

## Probe behaviour

`LinuxSandbox::probe()` now checks more than bare `unshare`: it writes uid/gid
maps and attempts `mount(MS_PRIVATE|/ )`. On AppArmor-restricted hosts it
should return `Unavailable` with an Ubuntu/sysctl hint instead of a false OK.
