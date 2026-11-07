//! Modes of `sandbox_fixture` that do not need an AppContainer.

use std::process::Command;

#[test]
fn fixture_read_prints_file_and_reports_a_missing_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "hello").expect("write");

    let ok = Command::new(env!("CARGO_BIN_EXE_sandbox_fixture"))
        .args(["read", file.to_str().expect("utf-8")])
        .output()
        .expect("spawn");
    assert!(
        ok.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ok.stdout), "hello");

    let missing = dir.path().join("absent.txt");
    let err = Command::new(env!("CARGO_BIN_EXE_sandbox_fixture"))
        .args(["read", missing.to_str().expect("utf-8")])
        .output()
        .expect("spawn");
    assert_eq!(err.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&err.stderr).contains("read failed"));
}
