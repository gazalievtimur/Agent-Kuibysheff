pub mod http_client;
pub mod oauth;
pub mod sse;
pub mod stdio_client;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::logging::LoggingError;
use crate::sandbox::SandboxError;

use self::oauth::BearerChallenge;

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
    #[error("home path `{path}` is not allowed: {error}")]
    HomePath { path: String, error: String },
    #[error("home filesystem operation `{operation}` failed for `{path}`: {source}")]
    HomeIo {
        operation: String,
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("local tools path `{path}` is not allowed: {error}")]
    LocalPath { path: String, error: String },
    #[error("local tools operation `{operation}` failed for `{path}`: {source}")]
    LocalIo {
        operation: String,
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("tool `{tool}` denied by access policy")]
    PolicyDenied { tool: String },
    #[error("sandbox unavailable: {reason}")]
    SandboxUnavailable { reason: String },
    #[error("sandbox failure")]
    Sandbox {
        #[source]
        source: SandboxError,
    },
    #[error("MCP server `{server}` actor channel closed")]
    ActorClosed { server: String },
}

/// Object-safe tool dispatch; `async_trait` is required because native `async fn` in traits is not
/// dyn-compatible for `Arc<dyn ToolExecutor>`.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value, Error>;
    fn available_tools(&self) -> Vec<String>;
}
