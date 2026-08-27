//! MCP client surface: registry, errors, and (crate-internal) transports.
//!
//! Stable for downstream use: [`McpRegistry`], [`McpError`] / [`Error`], and [`BearerChallenge`]
//! (present in error variants). HTTP/OAuth/SSE modules are `pub(crate)`.

pub(crate) mod http_client;
pub(crate) mod oauth;
pub(crate) mod sse;
pub(crate) mod stdio_client;

use thiserror::Error;

use crate::logging::LoggingError;
use crate::tool_api::{ExternalToolError, ToolError};

pub use crate::tool_api::ToolExecutor;
pub use oauth::BearerChallenge;
pub use stdio_client::{McpIsolationContext, McpRegistry};

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
    #[error("MCP call cancelled on server `{server}`")]
    Cancelled { server: String },
    #[error("MCP isolation denied on server `{server}`: {reason}")]
    IsolationDenied { server: String, reason: String },
    #[error("MCP sandbox unavailable for stdio server `{server}`: {reason}")]
    SandboxUnavailable { server: String, reason: String },
}

/// Backwards-compatible alias for the MCP-specific error type.
pub type Error = McpError;

impl McpError {
    /// Wraps a transport-level HTTP failure for `server`.
    #[must_use]
    pub(crate) fn http(server: impl Into<String>, source: reqwest::Error) -> Self {
        Self::Http {
            server: server.into(),
            source,
        }
    }
}

/// Extracts tool names from a `tools/list` JSON-RPC result object.
#[must_use]
pub(crate) fn tool_names_from_list_result(response: &serde_json::Value) -> Vec<String> {
    response
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    entry
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

impl From<McpError> for ExternalToolError {
    fn from(err: McpError) -> Self {
        match err {
            McpError::UnknownServer(server) => Self::UnknownServer(server),
            McpError::UnknownTool { server, tool } => Self::UnknownTool { server, tool },
            McpError::InvalidToolArguments { tool, error } => {
                Self::InvalidToolArguments { tool, error }
            }
            other => Self::Failed {
                message: other.to_string(),
            },
        }
    }
}

impl From<McpError> for ToolError {
    fn from(err: McpError) -> Self {
        Self::External(ExternalToolError::from(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_api::ToolError;

    #[test]
    fn mcp_error_converts_to_tool_error_at_boundary() {
        let err = ToolError::from(McpError::UnknownServer("docs".to_string()));
        assert!(
            matches!(
                err,
                ToolError::External(ExternalToolError::UnknownServer(ref s)) if s == "docs"
            ),
            "expected External::UnknownServer, got {err}"
        );

        let err = ToolError::from(McpError::UnknownTool {
            server: "docs".to_string(),
            tool: "search".to_string(),
        });
        assert!(
            matches!(
                err,
                ToolError::External(ExternalToolError::UnknownTool {
                    ref server,
                    ref tool
                }) if server == "docs" && tool == "search"
            ),
            "expected External::UnknownTool, got {err}"
        );

        let err = ToolError::from(McpError::ActorClosed {
            server: "docs".to_string(),
        });
        assert!(
            matches!(
                err,
                ToolError::External(ExternalToolError::Failed { ref message })
                    if message.contains("actor channel closed")
            ),
            "expected External::Failed with actor message, got {err}"
        );
    }

    #[test]
    fn question_mark_converts_mcp_error() {
        fn returns_mcp() -> Result<(), ToolError> {
            Err(McpError::UnknownServer("docs".to_string()))?;
            Ok(())
        }
        let err = returns_mcp().unwrap_err();
        assert!(
            matches!(
                err,
                ToolError::External(ExternalToolError::UnknownServer(ref s)) if s == "docs"
            ),
            "expected External::UnknownServer via ?, got {err}"
        );
    }
}
