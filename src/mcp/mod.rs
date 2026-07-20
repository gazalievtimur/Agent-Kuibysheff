pub mod stdio_client;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

/// Tool-layer error shared by MCP servers and builtin tool executors.
#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to spawn MCP server `{server}`: {source}")]
    Spawn {
        server: String,
        #[source]
        source: std::io::Error,
    },
    #[error("MCP server `{server}` missing stdio pipe: {pipe}")]
    MissingPipe { server: String, pipe: String },
    #[error("MCP protocol IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("MCP payload encode/decode failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP call timed out on server `{server}` for method `{method}`")]
    Timeout { server: String, method: String },
    #[error("MCP server `{server}` returned protocol error: {error}")]
    Protocol { server: String, error: String },
    #[error("unknown MCP server `{0}`")]
    UnknownServer(String),
    #[error("tool `{tool}` is not exposed by server `{server}`")]
    UnknownTool { server: String, tool: String },
    #[error("invalid arguments for tool `{tool}`: {error}")]
    InvalidToolArguments { tool: String, error: String },
    #[error("home path `{path}` is not allowed: {error}")]
    HomePath { path: String, error: String },
    #[error("home filesystem operation `{operation}` failed for `{path}`: {error}")]
    HomeIo {
        operation: String,
        path: String,
        error: String,
    },
    #[error("local tools path `{path}` is not allowed: {error}")]
    LocalPath { path: String, error: String },
    #[error("local tools operation `{operation}` failed for `{path}`: {error}")]
    LocalIo {
        operation: String,
        path: String,
        error: String,
    },
    #[error("MCP server `{server}` actor channel closed")]
    ActorClosed { server: String },
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, Error>;
    fn available_tools(&self) -> Vec<String>;
}
