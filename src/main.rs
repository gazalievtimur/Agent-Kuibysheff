use std::sync::Arc;

use clap::Parser;
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

#[tokio::main]
async fn main() {
    let output = match run().await {
        Ok(out) => out,
        Err(err) => RunOutput::error(err),
    };

    match serde_json::to_string_pretty(&output) {
        Ok(payload) => println!("{payload}"),
        Err(_) => println!(
            "{{\"result\":\"failed to serialize output\",\"usage\":{{\"iterations\":0,\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0,\"elapsed_ms\":0}},\"stop_reason\":\"error\",\"logs\":{{\"ai_log\":null,\"mcp_log\":null}}}}"
        ),
    }
}

async fn run() -> Result<RunOutput, String> {
    let cli = CliArgs::parse();
    let mut cfg = load_config(&cli.config).map_err(|err| err.to_string())?;
    apply_cli_overrides(&mut cfg, &cli);
    validate(&cfg).map_err(|err| err.to_string())?;
    if cli.prompt.trim().is_empty() {
        return Err("`--prompt` must not be empty".to_string());
    }

    let settings = load_settings(&cli.settings_dir).map_err(|err| err.to_string())?;
    let catalog = SkillsCatalog::parse(&settings.skills_source).map_err(|err| err.to_string())?;
    let allowed_tools = catalog.allowed_tool_set();
    let skill_prompt = catalog.build_prompt_fragment();
    let input_files_context =
        build_input_files_context(&cli.files).map_err(|err| err.to_string())?;
    let home = HomeFs::new(&cli.home)
        .await
        .map_err(|err| err.to_string())?;

    let loggers = Loggers::from_flags(
        cfg.logging.output_dir.as_ref(),
        cfg.logging.enable_ai_log,
        cfg.logging.enable_mcp_log,
    )
    .await
    .map_err(|err| err.to_string())?;

    let mcp = McpRegistry::connect_all(&cfg.mcp, loggers.mcp.clone())
        .await
        .map_err(|err| err.to_string())?;
    let provider = OpenAiCompatClient::new(cfg.provider.clone()).map_err(|err| err.to_string())?;
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
