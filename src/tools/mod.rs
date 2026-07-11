pub mod fs_home;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::mcp::{stdio_client::McpError, ToolExecutor};

use self::fs_home::HomeFs;

pub struct CompositeToolExecutor {
    home: HomeFs,
    external: Arc<dyn ToolExecutor>,
}

impl CompositeToolExecutor {
    pub fn new(home: HomeFs, external: Arc<dyn ToolExecutor>) -> Self {
        Self { home, external }
    }
}

#[async_trait]
impl ToolExecutor for CompositeToolExecutor {
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, McpError> {
        if server == "home" {
            self.home.call(tool, arguments).await
        } else {
            self.external.call_tool(server, tool, arguments).await
        }
    }

    fn available_tools(&self) -> Vec<String> {
        let mut tools = self.external.available_tools();
        tools.extend(
            ["home.list", "home.read", "home.write"]
                .into_iter()
                .map(str::to_string),
        );
        tools.sort();
        tools.dedup();
        tools
    }
}
