//! Process-level ACP stdio contract: redirected stdin/stdout stay protocol-only,
//! stderr is drained separately, and a long-lived child survives sequential prompts
//! (including repeated `init_tracing`).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, SessionNotification,
    StopReason, TextContent,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{ByteStreams, Client, ConnectionTo, Error as AcpError};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn acp_child_over_redirected_stdio_survives_sequential_prompts() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{
                "message": {
                    "content": "{\"done\":true,\"thought\":\"ok\",\"tool_calls\":[],\"result\":\"bridge-ok\"}"
                }
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        })))
        .mount(&mock)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let log_dir = tmp.path().join("logs");
    let home = tmp.path().join("home");
    let settings = tmp.path().join("settings");
    let config_path = tmp.path().join("agent-config.yaml");
    std::fs::create_dir_all(&settings).expect("settings dir");
    std::fs::create_dir_all(&log_dir).expect("log dir");

    std::fs::write(
        settings.join("master_prompt.md"),
        "You are a test agent. Reply with one JSON object only.\n",
    )
    .expect("master_prompt");
    std::fs::write(
        settings.join("skills.dsl"),
        r#"skill "workspace" {
  policy: "Use home tools only."
  allowed_tools: ["home.list", "home.read", "home.write"]
}
"#,
    )
    .expect("skills");
    std::fs::write(
        &config_path,
        format!(
            r#"provider:
  base_url: "{}/v1"
  model: "mock-model"
  api_key: "test-key"
  timeout_ms: 5000
  max_retries: 0
  retry_base_delay_ms: 1
  history:
    max_tail_messages: 10
    max_chars: 20000
mcp: []
limits:
  max_iterations: 2
  max_tokens: 1000
  max_duration_sec: 30
logging:
  enable_ai_log: false
  enable_mcp_log: false
  enable_chat_history: false
  sink:
    type: file
    path: "{}"
access:
  mode: legacy
"#,
            mock.uri(),
            log_dir.display().to_string().replace('\\', "/")
        ),
    )
    .expect("config");

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_agent_Kuibysheff"));
    let mut child = Command::new(&bin)
        .args([
            "acp",
            "--config",
            config_path.to_str().expect("utf-8 config"),
            "--settings-dir",
            settings.to_str().expect("utf-8 settings"),
            "--home",
            home.to_str().expect("utf-8 home"),
        ])
        .env("AGENT_LOG_DIR", &log_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn agent_Kuibysheff acp");

    let child_stdin = child.stdin.take().expect("child stdin");
    let child_stdout = child.stdout.take().expect("child stdout");
    let child_stderr = child.stderr.take().expect("child stderr");

    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_lines_task = Arc::clone(&stderr_lines);
    let stderr_drain = tokio::spawn(async move {
        let mut reader = BufReader::new(child_stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => stderr_lines_task
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(line.trim_end().to_string()),
                Err(_) => break,
            }
        }
    });

    let transport = ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());
    let notifications: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_notifications = Arc::clone(&notifications);

    let client_result = Client
        .builder()
        .name("bridge-test-client")
        .on_receive_notification(
            async move |notif: SessionNotification, _cx| {
                captured_notifications
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(serde_json::to_string(&notif).unwrap_or_default());
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(
            transport,
            |cx: ConnectionTo<agent_client_protocol::Agent>| {
                async move {
                    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;

                    let session = cx
                        .send_request(NewSessionRequest::new(std::env::temp_dir()))
                        .block_task()
                        .await?;

                    // Two sequential prompts exercise repeated init_tracing in one process.
                    for prompt_text in ["first bridge turn", "second bridge turn"] {
                        let response = cx
                            .send_request(PromptRequest::new(
                                session.session_id.clone(),
                                vec![ContentBlock::Text(TextContent::new(prompt_text))],
                            ))
                            .block_task()
                            .await?;
                        assert_eq!(
                            response.stop_reason,
                            StopReason::EndTurn,
                            "prompt `{prompt_text}` should complete"
                        );
                    }

                    Ok::<(), AcpError>(())
                }
            },
        )
        .await;

    client_result.expect("ACP client flow over redirected pipes");
    assert!(
        notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|notification| notification.contains("Usage:")
                && notification.contains("cost: unavailable")),
        "ACP final updates must include token and cost summary"
    );

    // Dropping the ByteStreams transport closes child stdin; the agent should exit cleanly.
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("agent did not exit after stdin EOF")
        .expect("wait on agent child");
    assert!(
        status.success(),
        "agent should exit 0 after clean stdin EOF, got {status:?}; stderr={:?}",
        stderr_lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    );

    let _ = tokio::time::timeout(Duration::from_secs(5), stderr_drain)
        .await
        .expect("stderr drain should finish after process exit");

    // Protocol must not leak onto stderr as JSON-RPC; diagnostics are fine.
    let captured = stderr_lines
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for line in captured.iter() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        assert!(
            !trimmed.starts_with('{') || !trimmed.contains("\"jsonrpc\""),
            "stderr must not contain ACP JSON-RPC frames: {line}"
        );
    }
}
