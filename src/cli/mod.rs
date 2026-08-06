//! CLI argument parsing with clap subcommands.

mod check;
mod init;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::billing::Money;

pub use check::CheckArgs;
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
    /// Create a new agent settings directory and starter config.
    Init(InitArgs),
    /// Check availability of resources configured for an agent.
    Check(CheckArgs),
}

/// Arguments for a single agent worker run.
#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[arg(long, value_name = "FILE")]
    pub config: PathBuf,

    #[arg(long, value_name = "DIR")]
    pub settings_dir: PathBuf,

    #[arg(long, value_name = "TEXT")]
    pub prompt: String,

    /// Optional orchestrator-supplied run identifier for invoice reconciliation.
    #[arg(long, value_name = "ID")]
    pub run_id: Option<String>,

    #[arg(long, value_name = "DIR")]
    pub home: PathBuf,

    /// Project root (e.g. 1C product folder). Relative `--config` /
    /// `--settings-dir` / `--home` resolve under `{project-root}/.kuibysheff/`.
    #[arg(long, value_name = "DIR")]
    pub project_root: Option<PathBuf>,

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
    #[arg(long, value_name = "FILE")]
    pub config: PathBuf,

    #[arg(long, value_name = "DIR")]
    pub settings_dir: PathBuf,

    #[arg(long, value_name = "DIR")]
    pub home: PathBuf,

    /// Fallback project root when the ACP client does not send session `cwd`.
    /// Relative `--config` / `--settings-dir` / `--home` resolve under
    /// `{project-root}/.kuibysheff/` (session `cwd` wins when non-empty).
    #[arg(long, value_name = "DIR")]
    pub project_root: Option<PathBuf>,

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
    fn parses_run_worker_inputs_and_multiple_files() {
        let cli = Cli::try_parse_from([
            "agent",
            "run",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--prompt",
            "do work",
            "--home",
            "home",
            "--files",
            "a.rs",
            "b.rs",
        ])
        .expect("parse args");

        let Commands::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(args.prompt, "do work");
        assert_eq!(
            args.files,
            vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]
        );
    }

    #[test]
    fn parses_explicit_run_subcommand() {
        let cli = Cli::try_parse_from([
            "agent",
            "run",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--prompt",
            "do work",
            "--home",
            "home",
            "--save-chat-history",
        ])
        .expect("parse args");

        let Commands::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert!(args.save_chat_history);
    }

    #[test]
    fn parses_run_id_and_exact_max_cost() {
        let cli = Cli::try_parse_from([
            "agent",
            "run",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--prompt",
            "do work",
            "--run-id",
            "invoice-row-42",
            "--home",
            "home",
            "--max-cost",
            "USD:0.00000894",
        ])
        .expect("parse args");
        let Commands::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(args.run_id.as_deref(), Some("invoice-row-42"));
        let max_cost = args.max_cost.expect("max cost");
        assert_eq!(max_cost.amount.to_string(), "0.00000894");
        assert_eq!(max_cost.currency, "USD");
    }

    #[test]
    fn parses_init_subcommand() {
        let cli = Cli::try_parse_from([
            "agent",
            "init",
            "my-agent",
            "--path",
            "/tmp/custom",
            "--force",
            "--interactive",
        ])
        .expect("parse init");

        let Commands::Init(args) = cli.command else {
            panic!("expected Init");
        };
        assert_eq!(args.agent_id, "my-agent");
        assert_eq!(args.path, Some(PathBuf::from("/tmp/custom")));
        assert!(args.force);
        assert!(args.interactive);
    }

    #[test]
    fn parses_check_subcommand() {
        let cli = Cli::try_parse_from([
            "agent",
            "check",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--skip-provider",
            "--skip-mcp",
            "--skip-sandbox",
        ])
        .expect("parse check");

        let Commands::Check(args) = cli.command else {
            panic!("expected Check");
        };
        assert_eq!(args.config, PathBuf::from("config.yaml"));
        assert_eq!(args.settings_dir, Some(PathBuf::from("settings")));
        assert!(args.skip_provider);
        assert!(args.skip_mcp);
        assert!(args.skip_sandbox);
    }

    #[test]
    fn root_help_lists_commands() {
        let err = Cli::try_parse_from(["agent", "--help"]).expect_err("help is an error exit");
        let rendered = err.to_string();
        assert!(rendered.contains("run"), "{rendered}");
        assert!(rendered.contains("acp"), "{rendered}");
        assert!(rendered.contains("init"), "{rendered}");
        assert!(rendered.contains("check"), "{rendered}");
        assert!(rendered.contains("help"), "{rendered}");
    }

    #[test]
    fn parses_acp_subcommand() {
        let cli = Cli::try_parse_from([
            "agent",
            "acp",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--home",
            "home",
            "--max-iterations",
            "3",
        ])
        .expect("parse acp");

        let Commands::Acp(args) = cli.command else {
            panic!("expected Acp");
        };
        assert_eq!(args.config, PathBuf::from("config.yaml"));
        assert_eq!(args.settings_dir, PathBuf::from("settings"));
        assert_eq!(args.home, PathBuf::from("home"));
        assert_eq!(args.max_iterations, Some(3));
        assert!(args.project_root.is_none());
    }

    #[test]
    fn parses_project_root_on_run_and_acp() {
        let run = Cli::try_parse_from([
            "agent",
            "run",
            "--config",
            "agents/a/agent-config.yaml",
            "--settings-dir",
            "agents/a",
            "--prompt",
            "x",
            "--home",
            "runs/h",
            "--project-root",
            "/proj",
        ])
        .expect("parse run");
        let Commands::Run(run_args) = run.command else {
            panic!("expected Run");
        };
        assert_eq!(run_args.project_root, Some(PathBuf::from("/proj")));

        let acp = Cli::try_parse_from([
            "agent",
            "acp",
            "--config",
            "agents/a/agent-config.yaml",
            "--settings-dir",
            "agents/a",
            "--home",
            "runs/h",
            "--project-root",
            "/proj",
        ])
        .expect("parse acp");
        let Commands::Acp(acp_args) = acp.command else {
            panic!("expected Acp");
        };
        assert_eq!(acp_args.project_root, Some(PathBuf::from("/proj")));
    }

    #[test]
    fn flat_flags_without_run_are_rejected() {
        let err = Cli::try_parse_from([
            "agent",
            "--config",
            "config.yaml",
            "--settings-dir",
            "settings",
            "--prompt",
            "do work",
            "--home",
            "home",
        ])
        .expect_err("flat flags require explicit run");
        let rendered = err.to_string();
        assert!(
            rendered.contains("unrecognized subcommand")
                || rendered.contains("unexpected argument")
                || rendered.contains("subcommand"),
            "{rendered}"
        );
    }
}
