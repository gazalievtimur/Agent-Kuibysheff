use std::sync::Arc;

use agent_Kuibyshev::access::{
    parse_tool_list, workspace_root_for_run, EffectiveToolPolicy, HomeFsPolicy, InputFilesPolicy,
    WorkspaceFsPolicy,
};
use agent_Kuibyshev::agent::{AgentEngine, AgentRunRequest};
use agent_Kuibyshev::cli::CliArgs;
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
use tracing::info;

fn main() {
    // Must run before Tokio so the Linux sandbox helper stays single-threaded.
    sandbox_linux::try_run_helper();

    agent_Kuibyshev::config::load_dotenv();

    let output = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(run())
    {
        Ok(out) => out,
        Err(err) => RunOutput::error(format!("{err:#}")),
    };

    match serde_json::to_string_pretty(&output) {
        Ok(payload) => println!("{payload}"),
        Err(_) => println!(
            "{{\"result\":\"failed to serialize output\",\"usage\":{{\"iterations\":0,\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0,\"elapsed_ms\":0}},\"stop_reason\":\"error\",\"logs\":{{\"ai_log\":null,\"mcp_log\":null,\"system_log\":null,\"chat_log\":null}}}}"
        ),
    }
}

async fn run() -> Result<RunOutput> {
    let cli = CliArgs::parse();

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
        effective,
    );

    let system_prompt = format!(
        "{master}\n\n{rules_section}{skills}\n\nRuntime rules:\n- Stay within configured limits.\n- The home directory is `{home}`. All file writes must use home.write and paths relative to this directory.\n- Input files are read-only context and are not copied into home automatically.\n- Builtin tools: home.list {{\"path\":\".\"}}, home.read {{\"path\":\"relative/path\",\"max_chars\":50000}}, home.write {{\"path\":\"relative/path\",\"content\":\"...\"}}, home.run {{\"program\":\"python\",\"args\":[\"solution.py\"],\"timeout_ms\":30000}}.\n- home.run executes argv (no shell) with cwd set to home; capture stdout/stderr/exit_code. The process runs inside the host OS sandbox (no generic network API).\n- Repository research tools: local_tools.search_docs {{\"query\":\"phrase\",\"max_results\":8}}, local_tools.read_file {{\"path\":\"relative/path\",\"max_chars\":6000}}. These read from the workspace root (`{workspace}`), not from home.\n- For coding tasks, write deliverables under out/ and create out/manifest.json according to the orchestrator contract in the supplied rules.\n- Use MCP tools when needed and allowed.\n- When the goal is achieved, return done=true and fill `result`.\n- Return strict JSON and never use markdown.",
        master = settings.master_prompt.trim(),
        rules_section = if settings.rules.trim().is_empty() {
            String::new()
        } else {
            format!("Rules:\n{}\n\n", settings.rules.trim())
        },
        skills = skill_prompt,
        home = cli.home.display(),
        workspace = workspace_root.display()
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
        })
        .await)
}
