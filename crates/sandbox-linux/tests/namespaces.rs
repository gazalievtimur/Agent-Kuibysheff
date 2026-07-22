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
    if LinuxSandbox::probe().is_err() {
        eprintln!("skip: Linux sandbox unavailable");
        return None;
    }
    Some(())
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
    let script = fixture_script(dir.path(), "sleep 30");
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
    let script = fixture_script(
        dir.path(),
        r#"
dev=$(cat /proc/net/dev)
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
    let script = fixture_script(dir.path(), "sh -c '(echo child)& wait'");
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
    let script = fixture_script(dir.path(), "sh -c '(echo child)& wait'");
    let mut request = base_request(dir.path().to_path_buf(), script, Vec::new());
    request.allow_children = true;
    let result = LinuxSandbox::run(&request).expect("sandboxed child-allow run");
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(result.stdout.contains("child"), "stdout={}", result.stdout);
}
