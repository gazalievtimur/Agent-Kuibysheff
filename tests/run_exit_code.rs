//! CLI exit-code contract for `run` (architecture review 08).

use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

#[test]
fn run_prints_error_json_and_exits_nonzero() {
    let tmp = tempdir().expect("tempdir");
    let project = tmp.path();

    let output = Command::new(env!("CARGO_BIN_EXE_agent_Kuibysheff"))
        .args([
            "run",
            "--project-root",
            project.to_str().expect("utf-8 path"),
            "--agent",
            "missing-agent",
            "--prompt",
            "should fail before the agent loop",
        ])
        .output()
        .expect("spawn agent_Kuibysheff");

    assert!(
        !output.status.success(),
        "expected non-zero exit for stop_reason=error, got {:?}; stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|err| {
        panic!("stdout must be RunOutput JSON ({err}): {stdout}");
    });
    assert_eq!(json["stop_reason"], "error");
    assert!(json["result"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(json["run_id"].as_str().is_some());
    assert_eq!(json["usage"]["cost"]["status"], "unavailable");
}

#[test]
fn check_keeps_its_own_exit_semantics() {
    let tmp = tempdir().expect("tempdir");
    let project = tmp.path();

    let output = Command::new(env!("CARGO_BIN_EXE_agent_Kuibysheff"))
        .args([
            "check",
            "--project-root",
            project.to_str().expect("utf-8 path"),
            "--agent",
            "missing-agent",
            "--skip-provider",
            "--skip-mcp",
            "--skip-sandbox",
        ])
        .output()
        .expect("spawn agent_Kuibysheff check");

    assert!(
        !output.status.success(),
        "check with missing agent profile should fail"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().starts_with('{'),
        "check must not emit RunOutput JSON"
    );
}
