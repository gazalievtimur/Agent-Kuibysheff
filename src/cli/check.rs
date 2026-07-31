use std::path::PathBuf;

use clap::Parser;

/// Arguments for checking configured agent resources.
#[derive(Debug, Parser)]
pub struct CheckArgs {
    /// Runtime agent config (YAML/JSON) with provider, MCP, access, and logging.
    #[arg(long, value_name = "FILE")]
    pub config: PathBuf,

    /// Optional settings directory (`master_prompt.md`, `skills.dsl`, `rules.md`).
    #[arg(long, value_name = "DIR")]
    pub settings_dir: Option<PathBuf>,

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
