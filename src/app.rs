//! CLI binary composition root.
//!
//! This module is the entry used by `main.rs`. It is public so the binary crate can call it,
//! but it is **not** part of the stable library facade for downstream dependents.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::Parser;
use tracing::{error, info};

use crate::access::{
    parse_tool_list, workspace_root_for_run, EffectiveToolPolicy, HomeFsPolicy, InputFilesPolicy,
    WorkspaceFsPolicy,
};
use crate::acp;
use crate::agent::{AgentEngine, AgentEventTx, AgentRunRequest, RunCancel};
use crate::cli::{AcpArgs, Cli, Commands, RunArgs};
use crate::commands;
use crate::config::{apply_limit_overrides, load_config, load_dotenv, validate, AppConfig};
use crate::context::build_input_files_context;
use crate::logging::{init_tracing, resolve_base_dir, Loggers};
use crate::mcp::stdio_client::McpRegistry;
use crate::output::{RunOutput, StopReason};
use crate::prompt::build_runtime_rules;
use crate::provider::openai_compat::OpenAiCompatClient;
use crate::sandbox::SandboxRunner;
use crate::settings::load_settings;
use crate::skills::dsl::SkillsCatalog;
use crate::tool_api::ToolExecutor;
use crate::tools::fs_home::HomeFs;
use crate::tools::local_tools::LocalTools;
use crate::tools::{CompositeToolExecutor, PolicyToolExecutor};

/// Shared inputs for one agent turn (CLI `run` or ACP `session/prompt`).
#[derive(Clone)]
pub struct AgentPromptArgs {
    pub config: PathBuf,
    pub settings_dir: PathBuf,
    pub home: PathBuf,
    pub prompt: String,
    pub files: Vec<PathBuf>,
    pub max_iterations: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_duration_sec: Option<u64>,
    pub save_chat_history: bool,
    pub cancel: RunCancel,
    pub events: AgentEventTx,
}

impl From<RunArgs> for AgentPromptArgs {
    fn from(cli: RunArgs) -> Self {
        Self {
            config: cli.config,
            settings_dir: cli.settings_dir,
            home: cli.home,
            prompt: cli.prompt,
            files: cli.files,
            max_iterations: cli.max_iterations,
            max_tokens: cli.max_tokens,
            max_duration_sec: cli.max_duration_sec,
            save_chat_history: cli.save_chat_history,
            cancel: RunCancel::new(),
            events: AgentEventTx::noop(),
        }
    }
}

/// Parse CLI args and dispatch to `run` / `init` / `check` / `acp`.
///
/// Call [`sandbox_linux::try_run_helper`] in `main` before this so the Linux helper stays
/// single-threaded ahead of the Tokio runtime.
#[must_use]
pub fn run() -> ExitCode {
    load_dotenv();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().ok();
            return ExitCode::from(err.exit_code().clamp(0, 255) as u8);
        }
    };

    match cli.command {
        Commands::Run(args) => run_worker(args),
        Commands::Acp(args) => run_acp(args),
        Commands::Init(args) => match commands::init::run(&args) {
            Ok(result) => {
                commands::init::print_success(&result);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
        Commands::Check(args) => match commands::check::run(&args) {
            Ok(report) => {
                commands::check::print_report(&report);
                if report.all_passed() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_worker(args: RunArgs) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: failed to start tokio runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    let output = match runtime.block_on(run_agent_prompt(AgentPromptArgs::from(args))) {
        Ok(out) => out,
        Err(err) => RunOutput::error(format!("{err:#}")),
    };

    match serde_json::to_string_pretty(&output) {
        Ok(payload) => {
            println!("{payload}");
            exit_code_for_run_output(&output)
        }
        Err(err) => {
            error!(error = %err, "failed to serialize RunOutput as JSON");
            println!(
                "{{\"result\":\"failed to serialize output\",\"usage\":{{\"iterations\":0,\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0,\"elapsed_ms\":0}},\"stop_reason\":\"error\",\"logs\":{{\"ai_log\":null,\"mcp_log\":null,\"system_log\":null,\"chat_log\":null}}}}"
            );
            ExitCode::FAILURE
        }
    }
}

fn run_acp(args: AcpArgs) -> ExitCode {
    // ACP stdio JSON-RPC must not share stdout with other prints; keep diagnostics on stderr.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: failed to start tokio runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(acp::run_acp_server(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: ACP server failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Exit code after printing `RunOutput`: non-zero only for `stop_reason: error`.
fn exit_code_for_run_output(output: &RunOutput) -> ExitCode {
    match output.stop_reason {
        StopReason::Error => ExitCode::FAILURE,
        StopReason::GoalReached | StopReason::LimitReached => ExitCode::SUCCESS,
    }
}

fn apply_prompt_overrides(cfg: &mut AppConfig, args: &AgentPromptArgs) {
    apply_limit_overrides(
        cfg,
        args.max_iterations,
        args.max_tokens,
        args.max_duration_sec,
        args.save_chat_history,
    );
}

/// Wire config/tools/provider and run one agent turn.
///
/// # Errors
///
/// Returns setup failures (config, settings, MCP, home init). Engine failures become
/// [`RunOutput`] with `stop_reason: error`.
pub async fn run_agent_prompt(args: AgentPromptArgs) -> Result<RunOutput> {
    let config_path = args.config.clone();
    let (mut cfg, access_policy) = tokio::task::spawn_blocking(move || load_config(&config_path))
        .await
        .context("loading config task")?
        .context("loading config")?;

    apply_prompt_overrides(&mut cfg, &args);
    validate(&cfg).context("validating config")?;
    if args.prompt.trim().is_empty() {
        bail!("prompt must not be empty");
    }

    let log_dir = resolve_base_dir(&cfg.logging).context("resolving log directory")?;
    init_tracing(&log_dir).context("initializing tracing")?;

    let loggers = Loggers::from_config(&cfg.logging)
        .await
        .context("initializing loggers")?;

    let settings_dir = args.settings_dir.clone();
    let settings = tokio::task::spawn_blocking(move || load_settings(&settings_dir))
        .await
        .context("loading settings task")?
        .with_context(|| format!("loading settings from `{}`", args.settings_dir.display()))?;

    let catalog = SkillsCatalog::parse(&settings.skills_source).context("parsing skills DSL")?;
    let skills_allowed = catalog.allowed_qualified_tools();
    let skill_prompt = catalog.build_prompt_fragment();

    let input_policy = InputFilesPolicy::from_access(&access_policy);
    let input_files = args.files.clone();
    let input_files_context =
        tokio::task::spawn_blocking(move || build_input_files_context(&input_files, &input_policy))
            .await
            .context("building input file context task")?
            .context("building input file context")?;

    let run_cancel = args.cancel;

    let home_policy = HomeFsPolicy::from_access(&access_policy);
    let sandbox = Arc::new(SandboxRunner::platform_default());
    let home = HomeFs::new(&args.home, home_policy, sandbox, run_cancel.clone())
        .await
        .with_context(|| format!("initializing home workspace `{}`", args.home.display()))?;

    let workspace_root = workspace_root_for_run(
        &access_policy,
        &std::env::current_dir().context("resolving current working directory")?,
    );
    let workspace_policy = WorkspaceFsPolicy::from_access(&access_policy);
    let local_tools = LocalTools::new(&workspace_root, workspace_policy)
        .await
        .with_context(|| {
            format!(
                "initializing local tools workspace `{}`",
                workspace_root.display()
            )
        })?;

    let mcp = McpRegistry::connect_all(&cfg.mcp, loggers.mcp.clone(), run_cancel.token().clone())
        .await
        .context("connecting MCP servers")?;
    let mcp_tools = parse_tool_list(mcp.available_tools())
        .map_err(|reason| anyhow::anyhow!("parsing MCP tool names: {reason}"))?;
    let effective = EffectiveToolPolicy::compile(&access_policy, &skills_allowed, mcp_tools);

    let provider =
        OpenAiCompatClient::new(cfg.provider.clone()).context("initializing provider")?;
    let tools = PolicyToolExecutor::new(
        Arc::new(CompositeToolExecutor::new(home, local_tools, Arc::new(mcp))),
        effective.clone(),
    );

    let runtime_rules =
        build_runtime_rules(&effective, &access_policy, &args.home, &workspace_root);

    let system_prompt = format!(
        "{master}\n\n{rules_section}{skills}\n\n{runtime_rules}",
        master = settings.master_prompt.trim(),
        rules_section = if settings.rules.trim().is_empty() {
            String::new()
        } else {
            format!("Rules:\n{}\n\n", settings.rules.trim())
        },
        skills = skill_prompt,
        runtime_rules = runtime_rules
    );

    info!(
        home = %args.home.display(),
        mcp_servers = cfg.mcp.len(),
        "starting agent run"
    );

    let engine = AgentEngine::new(Arc::new(provider), Arc::new(tools), loggers);
    Ok(engine
        .run(AgentRunRequest {
            prompt: args.prompt,
            system_prompt,
            input_files_context,
            limits: cfg.limits,
            history: cfg.provider.history.clone(),
            cancel: run_cancel,
            events: args.events,
        })
        .await)
}
