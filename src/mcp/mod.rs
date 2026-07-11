pub mod stdio_client;

use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, stdio_client::McpError>;
    fn available_tools(&self) -> Vec<String>;
}
