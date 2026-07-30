use std::path::PathBuf;

use clap::Parser;

/// Arguments for scaffolding a new agent settings directory.
#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Agent identifier used in the default path `./<agent-id>/`.
    pub agent_id: String,

    /// Target directory (defaults to `./<agent-id>`).
    #[arg(long, value_name = "DIR")]
    pub path: Option<PathBuf>,

    /// Overwrite known template files if the target directory already exists.
    #[arg(long)]
    pub force: bool,

    /// Prompt for provider and limits, then write them into the starter config.
    #[arg(short = 'i', long)]
    pub interactive: bool,
}
