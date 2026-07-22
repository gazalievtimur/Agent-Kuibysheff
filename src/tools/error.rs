use std::io;

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

/// Top-level error returned by any [`ToolExecutor`] implementation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ToolError {
    #[error("MCP error: {0}")]
    Mcp(#[from] crate::mcp::McpError),
    #[error("home filesystem error: {0}")]
    HomeFs(#[from] HomeFsError),
    #[error("local tools error: {0}")]
    LocalTools(#[from] LocalToolsError),
    #[error("policy error: {0}")]
    Policy(#[from] PolicyError),
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

    fn assert_mcp_error(err: ToolError) {
        assert!(
            matches!(err, ToolError::Mcp(crate::mcp::McpError::UnknownServer(ref s)) if s == "docs"),
            "expected Mcp::UnknownServer, got {err}"
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

        let mcp: crate::mcp::McpError = crate::mcp::McpError::UnknownServer("docs".to_string());
        assert_mcp_error(ToolError::from(mcp));
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

        fn returns_mcp() -> Result<(), ToolError> {
            Err(crate::mcp::McpError::UnknownServer("docs".to_string()))?;
            Ok(())
        }
        assert_mcp_error(returns_mcp().unwrap_err());
    }
}
