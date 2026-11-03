//! `kbshff mcp` — discover and call tools on saved MCP servers.

use clap::{Parser, Subcommand};

use super::config::ConfigFormat;
use super::AgentIdentityArgs;

/// Arguments for `kbshff mcp` (operator probe; not an orchestrator entrypoint).
#[derive(Debug, Parser)]
pub struct McpProbeArgs {
    #[command(flatten)]
    pub identity: AgentIdentityArgs,

    #[arg(long, value_enum, default_value_t = ConfigFormat::Json)]
    pub format: ConfigFormat,

    #[command(subcommand)]
    pub command: McpProbeCommand,
}

#[derive(Debug, Subcommand)]
pub enum McpProbeCommand {
    /// Connect each saved server (or one `--server`) and list tool schemas.
    Discover {
        #[arg(long, value_name = "NAME")]
        server: Option<String>,
    },
    /// Call one tool. Arguments JSON is read from stdin (`{}` when stdin is empty).
    Call {
        #[arg(long, value_name = "NAME")]
        server: String,
        #[arg(long, value_name = "NAME")]
        tool: String,
    },
}
