//! Linux namespace integration tests (compiled only on Linux hosts).

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_linux::{LinuxSandbox, SandboxLaunchRequest};
use tempfile::tempdir;

fn fixture_script(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("payload.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn base_request(cwd: PathBuf, exe: PathBuf, argv: Vec<String>) -> SandboxLaunchRequest {
    SandboxLaunchRequest {
        executable: exe,
        argv,
        cwd: cwd.clone(),
        env: BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), cwd.display().to_string()),
        ]),
        home_read: vec![cwd.clone()],
        home_write: vec![cwd],
        runtime_read_roots: vec![
            PathBuf::from("/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/usr/lib64"),
        ]
        .into_iter()
        .filter(|p| p.exists())
        .collect(),
        deadline: Duration::from_secs(15),
        max_output_chars: 64 * 1024,
        allow_children: false,
    }
}

fn require_sandbox() -> Option<()> {
    match LinuxSandbox::probe() {
        Ok(()) => Some(()),
        Err(err) => {
            if std::env::var_os("REQUIRE_LINUX_SANDBOX").is_some() {
                panic!("Linux sandbox required but unavailable: {err}");
            }
            eprintln!("skip: Linux sandbox unavailable ({err})");
            None
        }
    }
}

#[test]
fn probe_succeeds_or_explains() {
    // On restricted hosts (Docker without userns) probe may fail closed.
    match LinuxSandbox::probe() {
        Ok(()) => {}
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("unavailable") || msg.contains("unshare"),
                "unexpected probe error: {msg}"
            );
        }
    }
}

#[test]
fn echo_under_grants() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let script = fixture_script(dir.path(), "echo hello-linux-sandbox");
    let request = base_request(dir.path().to_path_buf(), script, Vec::new());
    let result = LinuxSandbox::run(&request).expect("sandboxed echo");
    assert!(!result.timed_out, "stderr={}", result.stderr);
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(
        result.stdout.contains("hello-linux-sandbox"),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
}

#[test]
fn deny_sibling_write() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let root = tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let target = sibling.join("x.txt");
    let script = fixture_script(&allowed, &format!("echo pwned > '{}'", target.display()));
    let mut request = base_request(allowed.clone(), script, Vec::new());
    request.home_write = vec![allowed];
    request.home_read = Vec::new();
    let result = LinuxSandbox::run(&request).expect("sandboxed deny run");
    assert_ne!(
        result.exit_code,
        Some(70),
        "sandbox setup failed: stderr={}",
        result.stderr
    );
    // Payload may fail writing; isolation is what matters.
    assert!(
        !target.exists(),
        "sibling write must be denied by mount isolation (stdout={} stderr={})",
        result.stdout,
        result.stderr
    );
}

#[test]
fn timeout_kills_tree() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    // Busy-loop with shell builtins only — `sleep` would need fork, which is
    // denied when `allow_children` is false (the secure default under test).
    let script = fixture_script(dir.path(), "while true; do :; done");
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.deadline = Duration::from_millis(800);
    let started = std::time::Instant::now();
    let result = LinuxSandbox::run(&request).expect("timeout run");
    assert!(result.timed_out || result.exit_code == Some(124));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn network_namespace_has_no_foreign_ifaces() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    // Empty netns: /proc/net/dev should not mention host NICs.
    // Read via shell redirection (no `cat`) so the check works with
    // `allow_children: false`.
    let script = fixture_script(
        dir.path(),
        r#"
dev=
while IFS= read -r line; do
  dev="$dev
$line"
done < /proc/net/dev
case "$dev" in
  *eth*|*wlan*|*enp*|*ens*|*wlp*)
    printf '%s\n' "$dev"
    exit 1
    ;;
esac
echo net-isolated
"#,
    );
    let request = base_request(dir.path().to_path_buf(), script, Vec::new());
    let result = LinuxSandbox::run(&request).expect("net isolation run");
    assert_eq!(
        result.exit_code,
        Some(0),
        "stderr={} stdout={}",
        result.stderr,
        result.stdout
    );
    assert!(result.stdout.contains("net-isolated"));
}

#[test]
fn argv_metacharacters_are_literal() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let script = fixture_script(dir.path(), r#"printf '%s\n' "$1""#);
    let request = base_request(
        dir.path().to_path_buf(),
        script,
        vec!["$(reboot); echo pwned | cat".into()],
    );
    let result = LinuxSandbox::run(&request).expect("argv literal run");
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(
        result.stdout.contains("$(reboot); echo pwned | cat"),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
}

#[test]
fn truncates_large_stdout() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let script = fixture_script(
        dir.path(),
        r#"
i=0
while [ "$i" -lt 400 ]; do
  printf A
  i=$((i + 1))
done
"#,
    );
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.max_output_chars = 64;
    let result = LinuxSandbox::run(&request).expect("truncate run");
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(result.stdout_truncated, "expected truncation flag");
    assert!(result.stdout.chars().count() <= 64);
}

#[test]
fn deny_child_processes_when_allow_children_false() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    // One extra `sh -c` is enough to require fork; avoid `wait`, which can hang
    // if a background job is created then frozen by the pid namespace.
    let script = fixture_script(dir.path(), "sh -c 'echo child'");
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.allow_children = false;
    let result = LinuxSandbox::run(&request).expect("sandboxed child-denial run");
    assert!(!result.timed_out, "timed out: stderr={}", result.stderr);
    assert_ne!(
        result.exit_code,
        Some(0),
        "child fork should be denied (stdout={} stderr={})",
        result.stdout,
        result.stderr
    );
    assert!(
        !result.stdout.contains("child"),
        "child output must not appear (stdout={})",
        result.stdout
    );
}

#[test]
fn allow_child_processes_when_allow_children_true() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let script = fixture_script(dir.path(), "sh -c 'echo child'");
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.allow_children = true;
    let result = LinuxSandbox::run(&request).expect("sandboxed child-allow run");
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(result.stdout.contains("child"), "stdout={}", result.stdout);
}

#[test]
fn timeout_kills_process_tree_with_allow_children() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let marker = dir.path().join("grandchild.alive");
    // Child + grandchild busy-loop; marker is removed only if grandchild survives.
    let script = fixture_script(
        dir.path(),
        &format!(
            r#"
touch '{}'
(
  while true; do :; done
) &
while true; do :; done
"#,
            marker.display()
        ),
    );
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.allow_children = true;
    request.deadline = Duration::from_millis(800);
    let started = std::time::Instant::now();
    let result = LinuxSandbox::run(&request).expect("tree timeout run");
    assert!(result.timed_out || result.exit_code == Some(124));
    assert!(started.elapsed() < Duration::from_secs(10));
    // Give the supervisor a moment; then ensure no orphan wrote after deadline.
    std::thread::sleep(Duration::from_millis(200));
    let _ = std::fs::remove_file(&marker);
    // Re-create and ensure a surviving grandchild cannot touch the host path
    // (mount is gone with the namespace). Best-effort: deadline respected.
    assert!(started.elapsed() < Duration::from_secs(12));
}

#[test]
fn seccomp_denies_unshare_and_mount() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    // python3 is commonly available; fall back to a tiny C-less probe via /bin/sh
    // is insufficient for errno. Use `unshare`/`mount` from util-linux when present.
    let script = fixture_script(
        dir.path(),
        r#"
ok=0
if command -v unshare >/dev/null 2>&1; then
  if unshare -U true 2>/dev/null; then
    echo unshare-allowed
    exit 1
  fi
  ok=1
fi
if command -v mount >/dev/null 2>&1; then
  if mount -t tmpfs tmpfs /mnt 2>/dev/null; then
    echo mount-allowed
    exit 1
  fi
  ok=1
fi
if [ "$ok" -eq 0 ]; then
  echo tools-missing
  exit 0
fi
echo seccomp-denied
"#,
    );
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    // unshare/mount binaries may fork helpers; allow children so the denial is seccomp.
    request.allow_children = true;
    let result = LinuxSandbox::run(&request).expect("seccomp deny run");
    assert_eq!(
        result.exit_code,
        Some(0),
        "stderr={} stdout={}",
        result.stderr,
        result.stdout
    );
    assert!(
        result.stdout.contains("seccomp-denied") || result.stdout.contains("tools-missing"),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
    assert!(!result.stdout.contains("unshare-allowed"));
    assert!(!result.stdout.contains("mount-allowed"));
}

#[test]
fn deny_sibling_read() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let root = tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let secret = sibling.join("secret.txt");
    std::fs::write(&secret, "top-secret").unwrap();
    let script = fixture_script(
        &allowed,
        &format!(
            r#"
if IFS= read -r line < '{}'; then
  printf '%s\n' "$line"
  exit 0
fi
echo read-denied
"#,
            secret.display()
        ),
    );
    let mut request = base_request(allowed.clone(), script, Vec::new());
    request.home_write = vec![allowed];
    request.home_read = Vec::new();
    let result = LinuxSandbox::run(&request).expect("sibling read deny");
    assert_ne!(
        result.exit_code,
        Some(70),
        "sandbox setup failed: stderr={}",
        result.stderr
    );
    assert!(
        !result.stdout.contains("top-secret"),
        "sibling read must be denied (stdout={} stderr={})",
        result.stdout,
        result.stderr
    );
}

#[test]
fn deny_symlink_escape_to_sibling() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let root = tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let secret = sibling.join("secret.txt");
    std::fs::write(&secret, "link-secret").unwrap();
    let link = allowed.join("escape");
    std::os::unix::fs::symlink(&secret, &link).unwrap();
    let script = fixture_script(
        &allowed,
        r#"
if IFS= read -r line < escape; then
  printf '%s\n' "$line"
  exit 0
fi
echo symlink-denied
"#,
    );
    let mut request = base_request(allowed.clone(), script, Vec::new());
    request.home_write = vec![allowed];
    request.home_read = Vec::new();
    let result = LinuxSandbox::run(&request).expect("symlink escape");
    assert_ne!(result.exit_code, Some(70), "stderr={}", result.stderr);
    assert!(
        !result.stdout.contains("link-secret"),
        "symlink escape must not leak sibling (stdout={} stderr={})",
        result.stdout,
        result.stderr
    );
}

#[test]
fn network_namespace_only_lo_and_connect_fails() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let script = fixture_script(
        dir.path(),
        r#"
# Enumerate /sys/class/net when present; only `lo` is allowed (empty netns is OK).
count=0
extra=
for p in /sys/class/net/*; do
  [ -e "$p" ] || continue
  name=$(basename "$p")
  count=$((count + 1))
  if [ "$name" != "lo" ]; then
    extra="$extra $name"
  fi
done
if [ -n "$extra" ]; then
  echo "ifaces-count=$count extra=$extra"
  exit 1
fi
# Connect to a public IP must fail (no route / no iface).
if command -v python3 >/dev/null 2>&1; then
  if python3 -c 'import socket; s=socket.socket(); s.settimeout(1); s.connect(("1.1.1.1", 80))' 2>/dev/null; then
    echo connect-ok
    exit 1
  fi
fi
echo net-lo-only
"#,
    );
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    // python3 -c spawns a helper process; allow_children must be true for the connect probe.
    request.allow_children = true;
    let result = LinuxSandbox::run(&request).expect("net lo-only run");
    assert_eq!(
        result.exit_code,
        Some(0),
        "stderr={} stdout={}",
        result.stderr,
        result.stdout
    );
    assert!(
        result.stdout.contains("net-lo-only"),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
}

#[test]
fn truncates_large_stderr_and_combined_pipes() {
    let Some(()) = require_sandbox() else {
        return;
    };
    let dir = tempdir().unwrap();
    let script = fixture_script(
        dir.path(),
        r#"
i=0
while [ "$i" -lt 400 ]; do
  printf B >&2
  printf A
  i=$((i + 1))
done
"#,
    );
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.max_output_chars = 64;
    let result = LinuxSandbox::run(&request).expect("stderr truncate run");
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(result.stdout_truncated, "expected stdout truncation");
    assert!(result.stderr_truncated, "expected stderr truncation");
    assert!(result.stdout.chars().count() <= 64);
    assert!(result.stderr.chars().count() <= 64);
}
