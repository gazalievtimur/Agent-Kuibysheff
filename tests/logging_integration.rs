use std::ffi::{OsStr, OsString};

use agent_Kuibysheff::config::{LogSinkConfig, LoggingConfig};
use agent_Kuibysheff::logging::{default_log_dir, init_tracing, Loggers, LoggingError};

/// Restores an environment variable to its previous value on drop.
struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[tokio::test]
async fn logging_pipeline_writes_trace_and_jsonl_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _log_dir = EnvRestore::set("AGENT_LOG_DIR", dir.path());

    let config = LoggingConfig {
        enable_ai_log: true,
        enable_mcp_log: true,
        enable_chat_history: false,
        output_dir: None,
        sink: LogSinkConfig::default(),
        ..Default::default()
    };

    let trace_path = init_tracing(dir.path()).expect("tracing");
    // Long-lived ACP processes call init_tracing once per session/prompt; same path must
    // stay idempotent and must not panic on a second install attempt.
    let again = init_tracing(dir.path()).expect("idempotent tracing");
    assert_eq!(trace_path, again);

    let other = tempfile::tempdir().expect("other tempdir");
    let switched = init_tracing(other.path());
    assert!(
        matches!(
            switched,
            Err(LoggingError::TracingAlreadyInitialized { .. })
        ),
        "expected TracingAlreadyInitialized, got {switched:?}"
    );

    let loggers = Loggers::from_config(&config).await.expect("loggers");

    if let Some(ai) = &loggers.ai {
        ai.write_event("ai_completion", serde_json::json!({"ok": true}))
            .await
            .expect("ai event");
    }
    if let Some(mcp) = &loggers.mcp {
        mcp.write_event("mcp_tool_call", serde_json::json!({"tool": "echo"}))
            .await
            .expect("mcp event");
    }

    tracing::info!("integration trace line");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let report = loggers.report();
    assert_eq!(report.system_log, Some(trace_path.display().to_string()));
    assert!(std::path::Path::new(report.ai_log.as_ref().expect("ai log")).exists());
    assert!(std::path::Path::new(report.mcp_log.as_ref().expect("mcp log")).exists());
    assert!(trace_path.exists());

    let trace_contents = std::fs::read_to_string(trace_path).expect("trace");
    assert!(trace_contents.contains("integration trace line"));

    // Explicit shutdown avoids Drop's 5s drain timeout on the runtime thread.
    loggers.shutdown().await;
}

#[test]
fn default_log_dir_is_under_dot_agent_kuibysheff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _home = EnvRestore::set("HOME", dir.path());
    let _userprofile = EnvRestore::set("USERPROFILE", dir.path());

    let log_dir = default_log_dir().expect("default log dir");
    assert!(log_dir.ends_with(".agent-kuibysheff/logs"));
}
