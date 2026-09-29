//! `ToolExecutor` for a connected MCP registry.

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{instrument, warn};

use super::registry::McpRegistry;
use crate::tool_api::{ToolError, ToolExecutor};

#[async_trait]
impl ToolExecutor for McpRegistry {
    #[instrument(skip(self, arguments), fields(server, tool))]
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        tracing::Span::current().record("server", server);
        tracing::Span::current().record("tool", tool);

        let arguments_for_log = self.logger.as_ref().map(|_| arguments.clone());
        let result = self.call_discovered_tool(server, tool, arguments).await?;

        // Runtime audit is soft: tools/call already succeeded; never map sink
        // failures to ToolError (would look like a tool failure to the model).
        if let Some(log) = &self.logger {
            if let Err(err) = log
                .write_event(
                    "mcp_tool_call",
                    json!({
                        "server": server,
                        "tool": tool,
                        "arguments": arguments_for_log,
                        "result": result,
                    }),
                )
                .await
            {
                warn!(
                    server,
                    tool,
                    error = ?err,
                    "MCP audit log write failed after successful tools/call; delivering tool result"
                );
            }
        }

        Ok(result)
    }

    fn available_tools(&self) -> Vec<String> {
        let mut entries = Vec::new();
        for (server, handle) in &self.servers {
            for tool in &handle.tools {
                entries.push(format!("{server}.{tool}"));
            }
        }
        entries.sort();
        entries
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::super::registry::McpRegistry;
    use crate::logging::sink::FailingEventSink;
    use crate::logging::SharedEventSink;
    use crate::tool_api::ToolExecutor;

    #[tokio::test]
    async fn call_tool_ok_when_audit_sink_fails() {
        let logger: SharedEventSink = Arc::new(FailingEventSink::new());
        let expected = json!({"content":[{"type":"text","text":"side-effect-done"}]});
        let registry =
            McpRegistry::with_stub_server("demo", "do_thing", expected.clone(), Some(logger));

        let result = registry
            .call_tool("demo", "do_thing", json!({"x": 1}))
            .await
            .expect("successful tools/call must not become ToolError on audit failure");
        assert_eq!(result, expected);

        registry.shutdown().await;
    }
}
