use std::sync::Arc;

use agent_Kuibyshev::agent::{AgentEngine, AgentRunRequest};
use agent_Kuibyshev::cli::CliArgs;
use agent_Kuibyshev::config::{apply_cli_overrides, load_config, validate};
use agent_Kuibyshev::context::build_input_files_context;
use agent_Kuibyshev::logging::Loggers;
use agent_Kuibyshev::mcp::stdio_client::McpRegistry;
use agent_Kuibyshev::output::RunOutput;
use agent_Kuibyshev::provider::openai_compat::OpenAiCompatClient;
use agent_Kuibyshev::settings::load_settings;
use agent_Kuibyshev::skills::dsl::SkillsCatalog;
use agent_Kuibyshev::tools::fs_home::HomeFs;
use agent_Kuibyshev::tools::CompositeToolExecutor;
use anyhow::{bail, Context, Result};
use clap::Parser;
use tracing::info;

#[tokio::main]
async fn main() {
    init_tracing();

    let output = match run().await {
        Ok(out) => out,
        Err(err) => RunOutput::error(format!("{err:#}")),
    };

    match serde_json::to_string_pretty(&output) {
        Ok(payload) => println!("{payload}"),
        Err(_) => println!(
            "{{\"result\":\"failed to serialize output\",\"usage\":{{\"iterations\":0,\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0,\"elapsed_ms\":0}},\"stop_reason\":\"error\",\"logs\":{{\"ai_log\":null,\"mcp_log\":null}}}}"
        ),
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn run() -> Result<RunOutput> {
    let cli = CliArgs::parse();

    let config_path = cli.config.clone();
    let mut cfg = tokio::task::spawn_blocking(move || load_config(&config_path))
        .await
        .context("loading config task")?
        .context("loading config")?;

    apply_cli_overrides(&mut cfg, &cli);
    validate(&cfg).context("validating config")?;
    if cli.prompt.trim().is_empty() {
        bail!("`--prompt` must not be empty");
    }

    let settings_dir = cli.settings_dir.clone();
    let settings = tokio::task::spawn_blocking(move || load_settings(&settings_dir))
        .await
        .context("loading settings task")?
        .with_context(|| format!("loading settings from `{}`", cli.settings_dir.display()))?;

    let catalog =
        SkillsCatalog::parse(&settings.skills_source).context("parsing skills DSL")?;
    let allowed_tools = Some(catalog.allowed_tool_set());
    let skill_prompt = catalog.build_prompt_fragment();

    let input_files = cli.files.clone();
    let input_files_context = tokio::task::spawn_blocking(move || build_input_files_context(&input_files))
        .await
        .context("building input file context task")?
        .context("building input file context")?;

    let home = HomeFs::new(&cli.home)
        .await
        .with_context(|| format!("initializing home workspace `{}`", cli.home.display()))?;

    let loggers = Loggers::from_flags(
        cfg.logging.output_dir.as_ref(),
        cfg.logging.enable_ai_log,
        cfg.logging.enable_mcp_log,
    )
    .await
    .context("initializing loggers")?;

    let mcp = McpRegistry::connect_all(&cfg.mcp, loggers.mcp.clone())
        .await
        .context("connecting MCP servers")?;
    let provider = OpenAiCompatClient::new(cfg.provider.clone()).context("initializing provider")?;
    let tools = CompositeToolExecutor::new(home, Arc::new(mcp));

    let system_prompt = format!(
        "{master}\n\n{rules_section}{skills}\n\nRuntime rules:\n- Stay within configured limits.\n- The home directory is `{home}`. All file writes must use home.write and paths relative to this directory.\n- Input files are read-only context and are not copied into home automatically.\n- Builtin tools: home.list {{\"path\":\".\"}}, home.read {{\"path\":\"relative/path\",\"max_chars\":50000}}, home.write {{\"path\":\"relative/path\",\"content\":\"...\"}}.\n- For coding tasks, write deliverables under out/ and create out/manifest.json according to the orchestrator contract in the supplied rules.\n- Use MCP tools when needed and allowed.\n- When the goal is achieved, return done=true and fill `result`.\n- Return strict JSON and never use markdown.",
        master = settings.master_prompt.trim(),
        rules_section = if settings.rules.trim().is_empty() {
            String::new()
        } else {
            format!("Rules:\n{}\n\n", settings.rules.trim())
        },
        skills = skill_prompt,
        home = cli.home.display()
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
            allowed_tools,
        })
        .await)
}
