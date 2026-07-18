pub mod fs_home;
pub mod local_tools;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::mcp::{stdio_client::McpError, ToolExecutor};

use self::fs_home::HomeFs;
use self::local_tools::LocalTools;

pub struct CompositeToolExecutor {
    home: HomeFs,
    local_tools: LocalTools,
    external: Arc<dyn ToolExecutor>,
}

impl CompositeToolExecutor {
    pub fn new(home: HomeFs, local_tools: LocalTools, external: Arc<dyn ToolExecutor>) -> Self {
        Self {
            home,
            local_tools,
            external,
        }
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
        match server {
            "home" => self.home.call(tool, arguments).await,
            "local_tools" => self.local_tools.call(tool, arguments).await,
            _ => self.external.call_tool(server, tool, arguments).await,
        }
    }

    fn available_tools(&self) -> Vec<String> {
        let mut tools = self.external.available_tools();
        tools.extend(
            [
                "home.list",
                "home.read",
                "home.write",
                "home.run",
                "local_tools.search_docs",
                "local_tools.read_file",
            ]
            .into_iter()
            .map(str::to_string),
        );
        tools.sort();
        tools.dedup();
        tools
    }
}
