//! Transport-neutral tool dispatch API.
//!
//! Owns [`ToolExecutor`] and [`ToolError`] so `mcp` and `tools` can both depend on this
//! module without forming a cycle.

use std::io;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::sandbox::SandboxError;

/// Errors from the `home.*` built-in filesystem tools.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HomeFsError {
    #[error("path `{path}` is not allowed: {error}")]
    PathDenied { path: String, error: String },
    #[error("operation `{operation}` failed for `{path}`: {source}")]
    Io {
        operation: String,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("sandbox unavailable: {reason}")]
    SandboxUnavailable { reason: String },
    #[error("sandbox failure")]
    Sandbox {
        #[source]
        source: SandboxError,
    },
    #[error("program `{program}` denied: {reason}")]
    ProgramDenied { program: String, reason: String },
    #[error("invalid arguments: {error}")]
    InvalidArguments { error: String },
    #[error("unknown tool `{tool}`")]
    UnknownTool { tool: String },
}

/// Errors from the `local_tools.*` built-in repository research tools.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LocalToolsError {
    #[error("path `{path}` is not allowed: {error}")]
    PathDenied { path: String, error: String },
    #[error("operation `{operation}` failed for `{path}`: {source}")]
    Io {
        operation: String,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("invalid arguments: {error}")]
    InvalidArguments { error: String },
    #[error("unknown tool `{tool}`")]
    UnknownTool { tool: String },
}

/// Access policy denials at the tool layer.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PolicyError {
    #[error("tool `{tool}` denied by access policy")]
    ToolDenied { tool: String },
}

/// Backend/transport failures surfaced through [`ToolExecutor`] without coupling to MCP types.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExternalToolError {
    #[error("unknown server `{0}`")]
    UnknownServer(String),
    #[error("tool `{tool}` is not exposed by server `{server}`")]
    UnknownTool { server: String, tool: String },
    #[error("invalid arguments for tool `{tool}`: {error}")]
    InvalidToolArguments { tool: String, error: String },
    #[error("{message}")]
    Failed { message: String },
}

/// Top-level error returned by any [`ToolExecutor`] implementation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ToolError {
    #[error("external tool error: {0}")]
    External(#[from] ExternalToolError),
    #[error("home filesystem error: {0}")]
    HomeFs(#[from] HomeFsError),
    #[error("local tools error: {0}")]
    LocalTools(#[from] LocalToolsError),
    #[error("policy error: {0}")]
    Policy(#[from] PolicyError),
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_home_fs_error(err: ToolError) {
        assert!(
            matches!(err, ToolError::HomeFs(HomeFsError::PathDenied { ref path, .. }) if path == "/etc/passwd"),
            "expected HomeFs::PathDenied, got {err}"
        );
    }

    fn assert_local_tools_error(err: ToolError) {
        assert!(
            matches!(err, ToolError::LocalTools(LocalToolsError::UnknownTool { ref tool }) if tool == "search_docs"),
            "expected LocalTools::UnknownTool, got {err}"
        );
    }

    fn assert_policy_error(err: ToolError) {
        assert!(
            matches!(err, ToolError::Policy(PolicyError::ToolDenied { ref tool }) if tool == "home.write"),
            "expected Policy::ToolDenied, got {err}"
        );
    }

    fn assert_external_error(err: ToolError) {
        assert!(
            matches!(err, ToolError::External(ExternalToolError::UnknownServer(ref s)) if s == "docs"),
            "expected External::UnknownServer, got {err}"
        );
    }

    #[test]
    fn from_conversions_wrap_correctly() {
        let home: HomeFsError = HomeFsError::PathDenied {
            path: "/etc/passwd".to_string(),
            error: "outside home".to_string(),
        };
        assert_home_fs_error(ToolError::from(home));

        let local: LocalToolsError = LocalToolsError::UnknownTool {
            tool: "search_docs".to_string(),
        };
        assert_local_tools_error(ToolError::from(local));

        let policy: PolicyError = PolicyError::ToolDenied {
            tool: "home.write".to_string(),
        };
        assert_policy_error(ToolError::from(policy));

        let external = ExternalToolError::UnknownServer("docs".to_string());
        assert_external_error(ToolError::from(external));
    }

    #[test]
    fn question_mark_converts_domain_errors() {
        fn returns_home() -> Result<(), ToolError> {
            Err(HomeFsError::PathDenied {
                path: "/etc/passwd".to_string(),
                error: "outside home".to_string(),
            })?;
            Ok(())
        }
        assert_home_fs_error(returns_home().unwrap_err());

        fn returns_local() -> Result<(), ToolError> {
            Err(LocalToolsError::UnknownTool {
                tool: "search_docs".to_string(),
            })?;
            Ok(())
        }
        assert_local_tools_error(returns_local().unwrap_err());

        fn returns_policy() -> Result<(), ToolError> {
            Err(PolicyError::ToolDenied {
                tool: "home.write".to_string(),
            })?;
            Ok(())
        }
        assert_policy_error(returns_policy().unwrap_err());

        fn returns_external() -> Result<(), ToolError> {
            Err(ExternalToolError::UnknownServer("docs".to_string()))?;
            Ok(())
        }
        assert_external_error(returns_external().unwrap_err());
    }
}
