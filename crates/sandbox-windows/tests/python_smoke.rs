use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use sandbox_windows::{SandboxLaunchRequest, WindowsSandbox};
use tempfile::tempdir;

#[test]
fn python_under_runtime_root_runs() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("solution.py"), "print(41+1)\n").unwrap();
    let py = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/aoc-sandbox-runtime/python");
    let exe = py.join("python.exe");
    if !exe.is_file() {
        eprintln!("skip: staged python missing at {}", exe.display());
        return;
    }
    WindowsSandbox::probe().expect("probe");
    let request = SandboxLaunchRequest {
        executable: exe,
        argv: vec!["solution.py".into()],
        cwd: dir.path().to_path_buf(),
        env: BTreeMap::new(),
        home_read: vec![dir.path().to_path_buf()],
        home_write: vec![dir.path().to_path_buf()],
        runtime_read_roots: vec![py],
        deadline: Duration::from_secs(30),
        max_output_chars: 64 * 1024,
        allow_children: false,
    };
    let result = WindowsSandbox::run(&request).expect("run");
    assert!(!result.timed_out, "stderr={}", result.stderr);
    assert_eq!(result.exit_code, Some(0), "stderr={} stdout={}", result.stderr, result.stdout);
    assert!(result.stdout.contains("42"), "stdout={} stderr={}", result.stdout, result.stderr);
}
