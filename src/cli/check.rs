use clap::Parser;

use super::AgentIdentityArgs;

/// Arguments for checking configured agent resources.
#[derive(Debug, Parser)]
pub struct CheckArgs {
    #[command(flatten)]
    pub identity: AgentIdentityArgs,

    /// Skip the live provider HTTP probe (still verifies the API key resolves).
    #[arg(long)]
    pub skip_provider: bool,

    /// Skip connecting to configured MCP servers.
    #[arg(long)]
    pub skip_mcp: bool,

    /// Skip the OS sandbox availability probe.
    #[arg(long)]
    pub skip_sandbox: bool,
}
