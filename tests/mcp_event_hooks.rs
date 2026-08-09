use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use agent_Kuibysheff::config::{McpServerConfig, McpStdioConfig, McpTransport};
use agent_Kuibysheff::event_mcp::{
    EventFailurePolicy, EventHandlerConfig, EventMcpConfig, EventMcpDispatcher,
    EventPipelineConfig, EventStage, PipelineEvents,
};
use agent_Kuibysheff::mcp::McpRegistry;
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn fixture_bin() -> String {
    env!("CARGO_BIN_EXE_mcp_stdio_fixture").to_string()
}

fn fixture_server(name: &str, handler_id: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        timeout_ms: 5_000,
        transport: McpTransport::Stdio(McpStdioConfig {
            command: fixture_bin(),
            args: vec!["event".to_string()],
            env: HashMap::from([("MCP_FIXTURE_HANDLER_ID".to_string(), handler_id.to_string())]),
            cwd: None,
        }),
    }
}

#[tokio::test]
async fn two_mcp_handlers_run_in_declared_order() {
    let cancel = CancellationToken::new();
    let registry = Arc::new(
        McpRegistry::connect_all(
            &[
                fixture_server("second_server", "second"),
                fixture_server("first_server", "first"),
            ],
            None,
            cancel.clone(),
        )
        .await
        .expect("connect fixtures"),
    );
    let event_config = EventMcpConfig {
        events: BTreeMap::from([(
            EventStage::ContextBeforeModel,
            EventPipelineConfig {
                handlers: vec![
                    EventHandlerConfig {
                        id: "first".to_string(),
                        target: "first_server.event_transform".to_string(),
                        timeout_ms: 2_000,
                        on_error: EventFailurePolicy::Abort,
                    },
                    EventHandlerConfig {
                        id: "second".to_string(),
                        target: "second_server.event_transform".to_string(),
                        timeout_ms: 2_000,
                        on_error: EventFailurePolicy::Abort,
                    },
                ],
            },
        )]),
        ..EventMcpConfig::default()
    };
    let dispatcher = EventMcpDispatcher::new(&event_config, registry.clone(), None)
        .expect("compile event handlers");

    let output = dispatcher
        .dispatch(
            EventStage::ContextBeforeModel,
            json!({ "trace": [] }),
            Some(1),
            &cancel,
        )
        .await
        .expect("dispatch event");

    assert_eq!(output["trace"], json!(["first", "second"]));

    drop(dispatcher);
    Arc::try_unwrap(registry)
        .ok()
        .expect("registry has no remaining owners")
        .shutdown()
        .await;
}
