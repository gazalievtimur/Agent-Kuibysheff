use std::process::ExitCode;
use std::sync::Arc;

use agent_Kuibyshev::access::{
    parse_tool_list, workspace_root_for_run, EffectiveToolPolicy, HomeFsPolicy, InputFilesPolicy,
    WorkspaceFsPolicy,
};
use agent_Kuibyshev::agent::{AgentEngine, AgentRunRequest};
use agent_Kuibyshev::cli::{Cli, Commands, RunArgs};
use agent_Kuibyshev::commands;
use agent_Kuibyshev::config::{apply_cli_overrides, load_config, validate};
use agent_Kuibyshev::context::build_input_files_context;
use agent_Kuibyshev::logging::{init_tracing, resolve_base_dir, Loggers};
use agent_Kuibyshev::mcp::stdio_client::McpRegistry;
use agent_Kuibyshev::mcp::ToolExecutor;
use agent_Kuibyshev::output::RunOutput;
use agent_Kuibyshev::provider::openai_compat::OpenAiCompatClient;
use agent_Kuibyshev::settings::load_settings;
use agent_Kuibyshev::skills::dsl::SkillsCatalog;
use agent_Kuibyshev::tools::fs_home::HomeFs;
use agent_Kuibyshev::tools::local_tools::LocalTools;
use agent_Kuibyshev::tools::{CompositeToolExecutor, PolicyToolExecutor};
use anyhow::{bail, Context, Result};
use clap::Parser;
use tracing::{error, info};

fn main() -> ExitCode {
    // Must run before Tokio so the Linux sandbox helper stays single-threaded.
    sandbox_linux::try_run_helper();

    agent_Kuibyshev::config::load_dotenv();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().ok();
            return ExitCode::from(err.exit_code().clamp(0, 255) as u8);
        }
    };

    match cli.command {
        Commands::Run(args) => run_worker(args),
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
    }
}

fn run_worker(args: RunArgs) -> ExitCode {
    let output = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(run(args))
    {
        Ok(out) => out,
        Err(err) => RunOutput::error(format!("{err:#}")),
    };

    match serde_json::to_string_pretty(&output) {
        Ok(payload) => {
            println!("{payload}");
            ExitCode::SUCCESS
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

async fn run(cli: RunArgs) -> Result<RunOutput> {
    let config_path = cli.config.clone();
    let (mut cfg, access_policy) = tokio::task::spawn_blocking(move || load_config(&config_path))
        .await
        .context("loading config task")?
        .context("loading config")?;

    apply_cli_overrides(&mut cfg, &cli);
    validate(&cfg).context("validating config")?;
    if cli.prompt.trim().is_empty() {
        bail!("`--prompt` must not be empty");
    }

    let log_dir = resolve_base_dir(&cfg.logging).context("resolving log directory")?;
    init_tracing(&log_dir).context("initializing tracing")?;

    let loggers = Loggers::from_config(&cfg.logging)
        .await
        .context("initializing loggers")?;

    let settings_dir = cli.settings_dir.clone();
    let settings = tokio::task::spawn_blocking(move || load_settings(&settings_dir))
        .await
        .context("loading settings task")?
        .with_context(|| format!("loading settings from `{}`", cli.settings_dir.display()))?;

    let catalog = SkillsCatalog::parse(&settings.skills_source).context("parsing skills DSL")?;
    let skills_allowed = catalog.allowed_qualified_tools();
    let skill_prompt = catalog.build_prompt_fragment();

    let input_policy = InputFilesPolicy::from_access(&access_policy);
    let input_files = cli.files.clone();
    let input_files_context =
        tokio::task::spawn_blocking(move || build_input_files_context(&input_files, &input_policy))
            .await
            .context("building input file context task")?
            .context("building input file context")?;

    let home_policy = HomeFsPolicy::from_access(&access_policy);
    let sandbox = Arc::new(agent_Kuibyshev::sandbox::SandboxRunner::platform_default());
    let home = HomeFs::new(&cli.home, home_policy, sandbox)
        .await
        .with_context(|| format!("initializing home workspace `{}`", cli.home.display()))?;

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

    let mcp = McpRegistry::connect_all(&cfg.mcp, loggers.mcp.clone())
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

    let runtime_rules = agent_Kuibyshev::prompt::build_runtime_rules(
        &effective,
        &access_policy,
        &cli.home,
        &workspace_root,
    );

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
        home = %cli.home.display(),
        mcp_servers = cfg.mcp.len(),
        "starting agent run"
    );

    let engine = AgentEngine::new(Arc::new(provider), Arc::new(tools), loggers);
    Ok(engine
        .run(AgentRunRequest {
            prompt: cli.prompt,
            system_prompt,
            input_files_context,
            limits: cfg.limits,
            history: cfg.provider.history.clone(),
        })
        .await)
}
