use std::path::PathBuf;

use clap::Parser;

/// Arguments for scaffolding a new agent profile.
#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Agent identifier (letters/digits any language, spaces, `_`, `-`).
    pub agent_id: String,

    /// Project root that owns `.kuibysheff/` (required).
    #[arg(long, value_name = "DIR")]
    pub project_root: PathBuf,

    /// Overwrite known template files if the profile already exists.
    #[arg(long)]
    pub force: bool,

    /// Prompt for provider and limits, then write them into the starter config.
    #[arg(short = 'i', long)]
    pub interactive: bool,
}
