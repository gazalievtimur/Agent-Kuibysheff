//! MCP stdio registry: child process, connected servers, and tool execution.
//!
//! Stable names ([`McpRegistry`], [`McpIsolationContext`]) stay at this module path.

mod executor;
mod process;
mod registry;

pub use registry::{McpIsolationContext, McpRegistry};
