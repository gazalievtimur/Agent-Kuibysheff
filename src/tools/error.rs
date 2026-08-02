//! Re-exports tool error types from [`crate::tool_api`] for callers that import via `tools::`.

pub use crate::tool_api::{
    ExternalToolError, HomeFsError, LocalToolsError, PolicyError, ToolError,
};
