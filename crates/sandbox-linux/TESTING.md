# Linux sandbox testing notes

Integration tests in `tests/namespaces.rs` need a real Linux host with
unprivileged user namespaces. They are `cfg(target_os = "linux")` and do not
run on Windows.

Miri CI runs only library unit tests (`cargo miri test -p sandbox-linux --lib`).
Namespace integration tests need host `stat`/`unshare`/`fork` and are not
Miri-isolatable.

## Lab Linux host (generic)

Use any Linux machine where you can enable unprivileged user namespaces.
Do not commit real hostnames, LAN IPs, usernames, or SSH identity paths.

| Item | Example placeholder |
|------|---------------------|
| SSH host alias | `YOUR_LINUX_HOST` (see `~/.ssh/config`) |
| Remote user | `youruser` |
| Identity file | `~/.ssh/id_ed25519` (or your key path) |
| Rust | rustup on `PATH` (`$HOME/.cargo/bin`) |

```sshconfig
Host YOUR_LINUX_HOST
    HostName your.linux.host.example
    User youruser
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 60
    ServerAliveCountMax 3
```

Smoke check:

```bash
ssh YOUR_LINUX_HOST 'uname -r; rustc --version; cat /proc/sys/kernel/unprivileged_userns_clone; cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns'
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

If passwordless sudo is not configured on the lab host, set the sysctl
interactively on the machine.

## Sync from Windows (dev machine)

Prefer a checkout path **without** spaces on the Linux host. Clone or copy the
repo to a local directory of your choice (examples below use placeholders).

PowerShell (from the Windows checkout):

```powershell
$repoRoot = "path\to\Agent-Kuibysheff"   # your local clone
$dest = "$env:TEMP\agent-kuibysheff-linux.tgz"
tar -czf $dest --exclude=target --exclude=.git --exclude=src/.code-index --exclude=.env `
  -C (Split-Path $repoRoot -Parent) (Split-Path $repoRoot -Leaf)
scp -o BatchMode=yes $dest YOUR_LINUX_HOST:/tmp/agent-kuibysheff-linux.tgz
ssh -o BatchMode=yes YOUR_LINUX_HOST @"
export PATH=`$HOME/.cargo/bin:`$PATH
rm -rf `$HOME/src/agent-kuibysheff-test
mkdir -p /tmp/agent-kuibysheff-extract
tar -xzf /tmp/agent-kuibysheff-linux.tgz -C /tmp/agent-kuibysheff-extract
# Move the extracted top-level directory into place (name may vary).
mv /tmp/agent-kuibysheff-extract/* `$HOME/src/agent-kuibysheff-test
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
# Provide API key via .env / agent-config.local.yaml / OPENAI_API_KEY
./scripts/aoc-regression.sh
./scripts/aoc-regression.sh --task-id 2024-01-1
./scripts/check.sh            # fmt/clippy/cargo test; AoC is opt-in (--aoc)
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
| `deny_sibling_read` | Sibling of write grant not readable |
| `deny_symlink_escape_to_sibling` | Symlink inside grant must not leak sibling content |
| `timeout_kills_tree` | Short deadline → `timed_out` or exit `124` |
| `timeout_kills_process_tree_with_allow_children` | Child+grandchild under deadline |
| `seccomp_denies_unshare_and_mount` | Denylist blocks mount/unshare from payload |
| `network_namespace_has_no_foreign_ifaces` | Empty netns: no host NICs in `/proc/net/dev` |
| `network_namespace_only_lo_and_connect_fails` | Only `lo` under `/sys/class/net`; connect fails |
| `argv_metacharacters_are_literal` | Shell metacharacters stay literal argv (not executed) |
| `truncates_large_stdout` | Large stdout sets truncation flag and respects `max_output_chars` |
| `truncates_large_stderr_and_combined_pipes` | stdout+stderr both truncated |
| `deny_child_processes_when_allow_children_false` | fork/clone denied when `allow_children=false` |
| `allow_child_processes_when_allow_children_true` | child output appears when allowed |

### Fail-closed CI

Set `REQUIRE_LINUX_SANDBOX=1` so probe failure panics instead of silent skip:

```bash
REQUIRE_LINUX_SANDBOX=1 cargo test -p sandbox-linux --test namespaces -- --nocapture --test-threads=1
```

PR CI runs this on `ubuntu-24.04` after relaxing
`kernel.apparmor_restrict_unprivileged_userns` for the runner VM.

Architecture: seccomp BPF is **x86_64-only** (`compile_error!` on other Linux arches).


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
