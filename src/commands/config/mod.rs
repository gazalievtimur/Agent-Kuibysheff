//! `config` management command dispatch.

mod access;
mod billing;
mod common;
mod event_mcp;
mod import;
mod limits;
mod mcp;
mod output;
mod prompt;
mod provider;
mod rules;
mod show;
mod skill;
mod tools;

use thiserror::Error;

use crate::cli::{ConfigArgs, ConfigCommand, ConfigFormat};
use crate::config::ConfigError;
use crate::project_paths::AgentPathError;
use crate::settings::SettingsError;
use crate::skills::dsl::SkillsError;

use self::common::CommonError;

/// Fatal errors from `config` management commands.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigCmdError {
    #[error(transparent)]
    AgentPath(#[from] AgentPathError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Skills(#[from] SkillsError),
    #[error(transparent)]
    Common(#[from] CommonError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

impl ConfigCmdError {
    #[must_use]
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

/// Run a `config` subcommand.
///
/// # Errors
///
/// Returns [`ConfigCmdError`] when resolution, load, validation, or mutation fails.
pub fn run(args: &ConfigArgs) -> Result<(), ConfigCmdError> {
    match run_inner(args) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = output::emit_error(args.format, &err.to_string());
            Err(err)
        }
    }
}

fn run_inner(args: &ConfigArgs) -> Result<(), ConfigCmdError> {
    match &args.command {
        ConfigCommand::Import(import_args) => import::run(args, import_args),
        ConfigCommand::Show => show::run(args),
        ConfigCommand::Provider(cmd) => provider::run(args, cmd),
        ConfigCommand::Limits(cmd) => limits::run(args, cmd),
        ConfigCommand::Access(cmd) => access::run(args, cmd),
        ConfigCommand::Mcp(cmd) => mcp::run(args, cmd),
        ConfigCommand::Billing(cmd) => billing::run(args, cmd),
        ConfigCommand::EventMcp(cmd) => event_mcp::run(args, cmd),
        ConfigCommand::Skill(cmd) => skill::run(args, cmd),
        ConfigCommand::Prompt(cmd) => prompt::run(args, cmd),
        ConfigCommand::Rules(cmd) => rules::run(args, cmd),
        ConfigCommand::Tools(cmd) => tools::run(args, cmd),
    }
}

/// Convenience for success envelopes used by handlers.
pub(crate) fn emit_ok<T: serde::Serialize>(
    format: ConfigFormat,
    resource: &str,
    action: &str,
    data: &T,
) -> Result<(), ConfigCmdError> {
    output::emit_ok(format, resource, action, data).map_err(ConfigCmdError::from)
}
