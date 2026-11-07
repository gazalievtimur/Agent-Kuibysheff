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
        max_processes: 1,
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
fn reads_existing_file_under_grant() {
    let dir = tempdir().unwrap();
    let note = dir.path().join("note.txt");
    std::fs::write(&note, "hello-existing").unwrap();
    let request = make_request(dir.path(), vec!["read".into(), note.display().to_string()]);
    let result = WindowsSandbox::run(&request).expect("read existing");
    assert_eq!(result.exit_code, Some(0), "stderr={}", result.stderr);
    assert!(
        result.stdout.contains("hello-existing"),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
}

#[test]
fn denies_read_outside_grant() {
    let root = tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let secret = sibling.join("secret.txt");
    std::fs::write(&secret, "top-secret").unwrap();

    let mut request = make_request(&allowed, vec!["read".into(), secret.display().to_string()]);
    request.home_write = vec![allowed.clone()];
    request.home_read = vec![allowed];

    let result = WindowsSandbox::run(&request).expect("read outside");
    assert_ne!(result.exit_code, Some(0), "stdout={}", result.stdout);
    assert!(
        !result.stdout.contains("top-secret"),
        "sibling read must be denied; stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
}

#[test]
fn git_status_in_workspace_when_installed() {
    let git = PathBuf::from(r"C:\Program Files\Git\cmd\git.exe");
    if !git.is_file() {
        eprintln!("skip: git is not installed at {}", git.display());
        return;
    }
    let dir = tempdir().unwrap();
    let mut request = SandboxLaunchRequest {
        executable: git,
        argv: vec!["init".into()],
        cwd: dir.path().to_path_buf(),
        env: BTreeMap::new(),
        home_read: vec![dir.path().to_path_buf()],
        home_write: vec![dir.path().to_path_buf()],
        runtime_read_roots: Vec::new(),
        deadline: Duration::from_secs(30),
        max_output_chars: 64 * 1024,
        allow_children: true,
        max_processes: 32,
    };
    let init = WindowsSandbox::run(&request).expect("git init");
    assert_eq!(
        init.exit_code,
        Some(0),
        "stdout={} stderr={}",
        init.stdout,
        init.stderr
    );
    std::fs::write(dir.path().join("note.txt"), "hi").unwrap();
    request.argv = vec!["status".into(), "--short".into()];
    let status = WindowsSandbox::run(&request).expect("git status");
    assert_eq!(
        status.exit_code,
        Some(0),
        "stdout={} stderr={}",
        status.stdout,
        status.stderr
    );
    assert!(
        status.stdout.contains("note.txt"),
        "stdout={} stderr={}",
        status.stdout,
        status.stderr
    );
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
