pub mod http_client;
pub mod oauth;
pub mod sse;
pub mod stdio_client;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::logging::LoggingError;
use crate::tools::ToolError;

use self::oauth::BearerChallenge;

/// MCP-specific error (JSON-RPC, transport, OAuth, server lifecycle).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum McpError {
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
    #[error("MCP HTTP error on server `{server}`: {source}")]
    Http {
        server: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("MCP OAuth error on server `{server}`: {error}")]
    OAuth { server: String, error: String },
    #[error("MCP server `{server}` requires authorization")]
    Unauthorized {
        server: String,
        challenge: Option<BearerChallenge>,
    },
    #[error("MCP session expired on server `{server}`")]
    SessionExpired { server: String },
    #[error("MCP call timed out on server `{server}` for method `{method}`")]
    Timeout { server: String, method: String },
    #[error("MCP server `{server}` returned protocol error: {error}")]
    Protocol { server: String, error: String },
    #[error("MCP logging failure on server `{server}`")]
    Logging {
        server: String,
        #[source]
        source: LoggingError,
    },
    #[error("unknown MCP server `{0}`")]
    UnknownServer(String),
    #[error("tool `{tool}` is not exposed by server `{server}`")]
    UnknownTool { server: String, tool: String },
    #[error("invalid arguments for tool `{tool}`: {error}")]
    InvalidToolArguments { tool: String, error: String },
    #[error("MCP server `{server}` actor channel closed")]
    ActorClosed { server: String },
}

/// Backwards-compatible alias for the MCP-specific error type.
pub type Error = McpError;

/// Object-safe tool dispatch; `async_trait` is required because native `async fn` in traits is not
/// dyn-compatible for `Arc<dyn ToolExecutor>`.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ToolError>;
    fn available_tools(&self) -> Vec<String>;
}
