//! `kbshff agent` — list profiles and preview home `out/` files.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use super::config::ConfigFormat;
use super::AgentIdentityArgs;

/// Arguments for `kbshff agent`.
#[derive(Debug, Parser)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List agent ids that have an `agent-config.yaml`.
    List(AgentListArgs),
    /// List or preview files under `homes/<agent>/out/`.
    Out(AgentOutArgs),
}

#[derive(Debug, Parser)]
pub struct AgentListArgs {
    /// Product/workspace directory that owns `.kuibysheff/`.
    #[arg(long, value_name = "DIR")]
    pub project_root: PathBuf,

    #[arg(long, value_enum, default_value_t = ConfigFormat::Text)]
    pub format: ConfigFormat,
}

#[derive(Debug, Parser)]
pub struct AgentOutArgs {
    #[command(flatten)]
    pub identity: AgentIdentityArgs,

    #[arg(long, value_enum, default_value_t = ConfigFormat::Text)]
    pub format: ConfigFormat,

    #[command(subcommand)]
    pub command: AgentOutCommand,
}

#[derive(Debug, Subcommand)]
pub enum AgentOutCommand {
    List,
    /// Preview one file. `name` must be a single path segment.
    Show {
        #[arg(value_name = "NAME")]
        name: String,
    },
}
