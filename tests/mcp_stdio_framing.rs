use std::collections::HashMap;

use agent_Kuibyshev::config::{McpServerConfig, McpStdioConfig, McpTransport};
use agent_Kuibyshev::mcp::stdio_client::McpRegistry;
use agent_Kuibyshev::mcp::{Error, ToolExecutor};

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
    assert_eq!(
        registry.available_tools(),
        vec!["fixture.echo".to_string()]
    );
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
        Ok(_) => panic!("Content-Length framing must fail"),
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
