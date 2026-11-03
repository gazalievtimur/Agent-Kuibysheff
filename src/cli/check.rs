use clap::Parser;

use super::config::ConfigFormat;
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

    /// `json` prints the [`crate::commands::check::CheckReport`] document.
    #[arg(long, value_enum, default_value_t = ConfigFormat::Text)]
    pub format: ConfigFormat,
}
