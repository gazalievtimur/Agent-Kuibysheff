//! Windows AppContainer integration tests (host must support AppContainers).

#![cfg(windows)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_windows::{SandboxLaunchRequest, WindowsSandbox};
use tempfile::tempdir;

fn fixture_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sandbox_fixture"))
}

fn make_request(cwd: &Path, argv: Vec<String>) -> SandboxLaunchRequest {
    let exe = fixture_exe();
    // Copy fixture into the granted cwd so we only ACL user-controlled trees.
    let local_exe = cwd.join("sandbox_fixture.exe");
    std::fs::copy(&exe, &local_exe).expect("copy fixture");
    SandboxLaunchRequest {
        executable: local_exe,
        argv,
        cwd: cwd.to_path_buf(),
        env: BTreeMap::new(),
        home_read: vec![cwd.to_path_buf()],
        home_write: vec![cwd.to_path_buf()],
        runtime_read_roots: Vec::new(),
        deadline: Duration::from_secs(15),
        max_output_chars: 64 * 1024,
        allow_children: false,
    }
}

#[test]
fn probe_succeeds_on_windows() {
    WindowsSandbox::probe().expect("AppContainer probe should succeed");
}

#[test]
fn echo_under_grants() {
    let dir = tempdir().unwrap();
    let request = make_request(dir.path(), vec!["echo".into(), "hello-sandbox".into()]);
    let result = WindowsSandbox::run(&request).expect("sandboxed echo");
    assert!(!result.timed_out, "stderr={}", result.stderr);
    assert!(
        result.stdout.contains("hello-sandbox"),
        "stdout={} stderr={} code={:?}",
        result.stdout,
        result.stderr,
        result.exit_code
    );
}

#[test]
fn deny_sibling_write() {
    let root = tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();

    let target = sibling.join("x.txt");
    let mut request = make_request(
        &allowed,
        vec!["write".into(), target.display().to_string(), "pwned".into()],
    );
    request.home_write = vec![allowed];
    request.home_read = Vec::new();

    let result = WindowsSandbox::run(&request).expect("run should complete");
    assert!(
        !target.exists(),
        "sibling write must be denied; stdout={} stderr={} code={:?}",
        result.stdout,
        result.stderr,
        result.exit_code
    );
}

#[test]
fn timeout_kills_job() {
    let dir = tempdir().unwrap();
    let mut request = make_request(dir.path(), vec!["sleep-ms".into(), "30000".into()]);
    request.deadline = Duration::from_millis(800);
    let started = std::time::Instant::now();
    let result = WindowsSandbox::run(&request).expect("timeout run");
    assert!(
        result.timed_out,
        "expected timeout; stderr={}",
        result.stderr
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "job kill took too long: {:?}",
        started.elapsed()
    );
}

#[test]
fn argv_metacharacters_are_literal() {
    let dir = tempdir().unwrap();
    let request = make_request(
        dir.path(),
        vec!["echo".into(), "$(reboot); echo pwned | cat".into()],
    );
    let result = WindowsSandbox::run(&request).expect("argv literal");
    assert!(
        result.stdout.contains("$(reboot); echo pwned | cat"),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
}

#[test]
fn truncates_large_stdout() {
    let dir = tempdir().unwrap();
    let mut request = make_request(dir.path(), vec!["emit-bytes".into(), "4096".into()]);
    request.max_output_chars = 64;
    let result = WindowsSandbox::run(&request).expect("truncate");
    assert!(result.stdout_truncated);
    assert!(result.stdout.chars().count() <= 64);
}

#[test]
fn loopback_connect_is_denied() {
    let dir = tempdir().unwrap();
    let request = make_request(dir.path(), vec!["connect-loopback".into(), "9".into()]);
    let result = WindowsSandbox::run(&request).expect("loopback probe");
    assert!(
        !result.stdout.contains("connected"),
        "loopback must be blocked; stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
    assert_ne!(result.exit_code, Some(0));
}
