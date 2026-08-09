//! CLI argument parsing with clap subcommands.

mod check;
mod config;
mod init;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::billing::Money;

pub use check::CheckArgs;
pub use config::{
    AccessCmd, BillingCmd, BuiltinsCmd, ConfigArgs, ConfigCommand, ConfigFormat, EventMcpCmd,
    ImportArgs, LimitsCmd, McpCmd, PromptCmd, ProviderCmd, RulesCmd, SkillCmd, SkillToolsCmd,
    ToolsCmd,
};
pub use init::InitArgs;

/// Top-level CLI.
#[derive(Debug, Parser)]
#[command(
    name = "agent_Kuibysheff",
    version,
    about = "agent_Kuibysheff CLI worker and agent tooling",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run one worker iteration (orchestrator entrypoint).
    Run(RunArgs),
    /// Serve Agent Client Protocol (ACP) over stdio for IDE hosts.
    Acp(AcpArgs),
    /// Create a new agent profile under `.kuibysheff/protected/agents/<id>/`.
    Init(InitArgs),
    /// Check availability of resources for a configured agent profile.
    Check(CheckArgs),
    /// Manage agent settings (CRUD) without exposing storage paths.
    Config(ConfigArgs),
}

/// Shared identity: project root + agent id (canonical store under `.kuibysheff`).
#[derive(Debug, Clone, Parser)]
pub struct AgentIdentityArgs {
    /// Product/workspace directory that owns `.kuibysheff/`.
    #[arg(long, value_name = "DIR")]
    pub project_root: PathBuf,

    /// Agent id (`[a-z0-9][a-z0-9_-]*`). Profile lives under
    /// `.kuibysheff/protected/agents/<id>/`.
    #[arg(long, value_name = "ID")]
    pub agent: String,
}

/// Arguments for a single agent worker run.
#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[command(flatten)]
    pub identity: AgentIdentityArgs,

    #[arg(long, value_name = "TEXT")]
    pub prompt: String,

    /// Optional orchestrator-supplied run identifier for invoice reconciliation.
    #[arg(long, value_name = "ID")]
    pub run_id: Option<String>,

    /// Optional home under `.kuibysheff/` (relative). Default: `homes/<agent>`.
    /// Absolute paths and paths under `protected/` are rejected.
    #[arg(long, value_name = "DIR")]
    pub home: Option<PathBuf>,

    #[arg(long, value_name = "PATH", num_args = 1.., action = clap::ArgAction::Append)]
    pub files: Vec<PathBuf>,

    #[arg(long)]
    pub max_iterations: Option<u32>,

    #[arg(long)]
    pub max_tokens: Option<u64>,

    #[arg(long)]
    pub max_duration_sec: Option<u64>,

    /// Override the monetary run limit (`CURRENCY:AMOUNT`, for example `USD:1.00`).
    #[arg(long, value_name = "CURRENCY:AMOUNT")]
    pub max_cost: Option<Money>,

    /// Persist the full unpruned chat transcript for this run.
    #[arg(long)]
    pub save_chat_history: bool,
}

/// Arguments for the ACP stdio server (no prompt; IDE drives turns).
#[derive(Debug, Clone, Parser)]
pub struct AcpArgs {
    /// Agent id. Profile under `.kuibysheff/protected/agents/<id>/`.
    #[arg(long, value_name = "ID")]
    pub agent: String,

    /// Fallback project root when the ACP client does not send session `cwd`.
    #[arg(long, value_name = "DIR")]
    pub project_root: Option<PathBuf>,

    /// Optional home under `.kuibysheff/` (relative). Default: `homes/<agent>`.
    #[arg(long, value_name = "DIR")]
    pub home: Option<PathBuf>,

    #[arg(long)]
    pub max_iterations: Option<u32>,

    #[arg(long)]
    pub max_tokens: Option<u64>,

    #[arg(long)]
    pub max_duration_sec: Option<u64>,

    /// Override the monetary per-prompt limit (`CURRENCY:AMOUNT`).
    #[arg(long, value_name = "CURRENCY:AMOUNT")]
    pub max_cost: Option<Money>,

    /// Persist the full unpruned chat transcript for each prompt turn.
    #[arg(long)]
    pub save_chat_history: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_with_agent_identity() {
        let cli = Cli::try_parse_from([
            "agent",
            "run",
            "--project-root",
            "/proj",
            "--agent",
            "demo",
            "--prompt",
            "do work",
            "--files",
            "a.rs",
            "b.rs",
        ])
        .expect("parse args");

        let Commands::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(args.prompt, "do work");
        assert_eq!(args.identity.agent, "demo");
        assert_eq!(args.identity.project_root, PathBuf::from("/proj"));
        assert_eq!(
            args.files,
            vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]
        );
    }

    #[test]
    fn rejects_legacy_config_flag_on_run() {
        let err = Cli::try_parse_from([
            "agent",
            "run",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--prompt",
            "x",
            "--home",
            "home",
        ])
        .expect_err("legacy flags removed");
        let rendered = err.to_string();
        assert!(
            rendered.contains("unexpected argument")
                || rendered.contains("unrecognized")
                || rendered.contains("--config"),
            "{rendered}"
        );
    }

    #[test]
    fn parses_init_with_project_root() {
        let cli = Cli::try_parse_from([
            "agent",
            "init",
            "my-agent",
            "--project-root",
            "/proj",
            "--force",
            "--interactive",
        ])
        .expect("parse init");

        let Commands::Init(args) = cli.command else {
            panic!("expected Init");
        };
        assert_eq!(args.agent_id, "my-agent");
        assert_eq!(args.project_root, PathBuf::from("/proj"));
        assert!(args.force);
        assert!(args.interactive);
    }

    #[test]
    fn parses_check_with_agent() {
        let cli = Cli::try_parse_from([
            "agent",
            "check",
            "--project-root",
            "/proj",
            "--agent",
            "demo",
            "--skip-provider",
            "--skip-mcp",
            "--skip-sandbox",
        ])
        .expect("parse check");

        let Commands::Check(args) = cli.command else {
            panic!("expected Check");
        };
        assert_eq!(args.identity.agent, "demo");
        assert!(args.skip_provider);
    }

    #[test]
    fn parses_config_show() {
        let cli = Cli::try_parse_from([
            "agent",
            "config",
            "--project-root",
            "/proj",
            "--agent",
            "demo",
            "--format",
            "json",
            "show",
        ])
        .expect("parse config");
        let Commands::Config(args) = cli.command else {
            panic!("expected Config");
        };
        assert_eq!(args.format, ConfigFormat::Json);
        assert_eq!(args.identity.agent, "demo");
    }

    #[test]
    fn root_help_lists_commands() {
        let err = Cli::try_parse_from(["agent", "--help"]).expect_err("help is an error exit");
        let rendered = err.to_string();
        assert!(rendered.contains("run"), "{rendered}");
        assert!(rendered.contains("acp"), "{rendered}");
        assert!(rendered.contains("init"), "{rendered}");
        assert!(rendered.contains("check"), "{rendered}");
        assert!(rendered.contains("config"), "{rendered}");
    }

    #[test]
    fn parses_acp_subcommand() {
        let cli = Cli::try_parse_from([
            "agent",
            "acp",
            "--project-root",
            "/proj",
            "--agent",
            "demo",
            "--max-iterations",
            "3",
        ])
        .expect("parse acp");

        let Commands::Acp(args) = cli.command else {
            panic!("expected Acp");
        };
        assert_eq!(args.project_root, Some(PathBuf::from("/proj")));
        assert_eq!(args.agent, "demo");
        assert_eq!(args.max_iterations, Some(3));
    }
}
