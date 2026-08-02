use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use agent_Kuibyshev::config::{McpServerConfig, McpStdioConfig, McpTransport};
use agent_Kuibyshev::mcp::stdio_client::McpRegistry;
use agent_Kuibyshev::mcp::Error;
use agent_Kuibyshev::tool_api::ToolExecutor;
use tokio::time::sleep;

fn fixture_bin() -> String {
    env!("CARGO_BIN_EXE_mcp_stdio_fixture").to_string()
}

#[tokio::test]
async fn mcp_stdio_ndjson_connect_lists_tools() {
    let configs = [McpServerConfig {
        name: "fixture".to_string(),
        timeout_ms: 5_000,
        transport: McpTransport::Stdio(McpStdioConfig {
            command: fixture_bin(),
            args: vec![],
            env: HashMap::new(),
        }),
    }];

    let registry = McpRegistry::connect_all(&configs, None)
        .await
        .expect("connect NDJSON fixture");
    assert_eq!(registry.available_tools(), vec!["fixture.echo".to_string()]);
    registry.shutdown().await;
}

#[tokio::test]
async fn mcp_stdio_rejects_content_length_framing() {
    let configs = [McpServerConfig {
        name: "legacy".to_string(),
        timeout_ms: 5_000,
        transport: McpTransport::Stdio(McpStdioConfig {
            command: fixture_bin(),
            args: vec!["content-length".to_string()],
            env: HashMap::new(),
        }),
    }];

    let err = match McpRegistry::connect_all(&configs, None).await {
        Ok(registry) => {
            registry.shutdown().await;
            panic!("Content-Length framing must fail");
        }
        Err(err) => err,
    };
    match err {
        Error::Protocol { server, error } => {
            assert_eq!(server, "legacy");
            assert!(
                error.contains("expected NDJSON, got Content-Length framing"),
                "unexpected protocol error: {error}"
            );
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn mcp_stdio_registry_shutdown_terminates_fixture() {
    let alive_path: PathBuf = std::env::temp_dir().join(format!(
        "mcp_stdio_alive_{}_{}.flag",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&alive_path);

    let mut env = HashMap::new();
    env.insert(
        "MCP_FIXTURE_ALIVE_FILE".to_string(),
        alive_path.to_string_lossy().into_owned(),
    );

    let configs = [McpServerConfig {
        name: "fixture".to_string(),
        timeout_ms: 5_000,
        transport: McpTransport::Stdio(McpStdioConfig {
            command: fixture_bin(),
            args: vec![],
            env,
        }),
    }];

    let registry = McpRegistry::connect_all(&configs, None)
        .await
        .expect("connect NDJSON fixture");

    // Fixture should have created the alive marker shortly after spawn.
    let mut saw_alive = false;
    for _ in 0..50 {
        if alive_path.exists() {
            saw_alive = true;
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(
        saw_alive,
        "expected alive file at {} while fixture runs",
        alive_path.display()
    );

    registry.shutdown().await;

    let mut gone = false;
    for _ in 0..50 {
        if !alive_path.exists() {
            gone = true;
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(
        gone,
        "alive file {} should be removed when fixture exits",
        alive_path.display()
    );
}
