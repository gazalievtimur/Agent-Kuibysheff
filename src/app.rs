//! CLI binary composition root.
//!
//! This module is the entry used by `main.rs`. It is public so the binary crate can call it,
//! but it is **not** part of the stable library facade for downstream dependents.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::Parser;
use tracing::{error, info, warn};

use crate::a2a;
use crate::access::{
    parse_tool_list, workspace_root_for_run, EffectiveToolPolicy, HomeFsPolicy, InputFilesPolicy,
    WorkspaceFsPolicy,
};
use crate::acp;
use crate::agent::{AgentEngine, AgentEventTx, AgentRunRequest, RunCancel};
use crate::billing::{
    CatalogCostResolver, CostResolver, CostResolverChain, McpCostResolver, Money, PricingCatalog,
    ProviderReportedCostResolver, UnavailableCostResolver,
};
use crate::cli::{A2aArgs, AcpArgs, Cli, Commands, RunArgs};
use crate::commands;
use crate::config::{
    apply_limit_overrides, load_config, load_dotenv_layered, validate, AppConfig, BillingSource,
    ConfigSafetyValidator,
};
use crate::context::build_input_files_context;
use crate::event_mcp::EventMcpDispatcher;
use crate::logging::{init_tracing, resolve_base_dir, Loggers, SharedEventSink};
use crate::mcp::stdio_client::McpRegistry;
use crate::output::{RunOutput, StopReason};
use crate::project_paths::{resolve_agent_identity, resolve_config_path_for_dotenv};
use crate::prompt::build_runtime_rules;
use crate::provider::openai_compat::{OpenAiCompatClient, ProviderAccountingOptions};
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
    pub project_root: Option<PathBuf>,
    pub agent_id: String,
    pub prompt: String,
    pub run_id: Option<String>,
    pub files: Vec<PathBuf>,
    pub max_iterations: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_duration_sec: Option<u64>,
    pub max_cost: Option<Money>,
    pub save_chat_history: bool,
    pub cancel: RunCancel,
    pub events: AgentEventTx,
}

impl TryFrom<RunArgs> for AgentPromptArgs {
    type Error = anyhow::Error;

    fn try_from(cli: RunArgs) -> Result<Self> {
        let paths = resolve_agent_identity(
            &cli.identity.project_root,
            &cli.identity.agent,
            cli.home.as_deref(),
        )?;
        Ok(Self {
            config: paths.config,
            settings_dir: paths.settings_dir,
            home: paths.home,
            project_root: Some(paths.project_root),
            agent_id: paths.agent_id,
            prompt: cli.prompt,
            run_id: cli.run_id,
            files: cli.files,
            max_iterations: cli.max_iterations,
            max_tokens: cli.max_tokens,
            max_duration_sec: cli.max_duration_sec,
            max_cost: cli.max_cost,
            save_chat_history: cli.save_chat_history,
            cancel: RunCancel::new(),
            events: AgentEventTx::noop(),
        })
    }
}

/// Parse CLI args and dispatch to `run` / `init` / `check` / `acp` / `a2a` / `config` / wizard.
///
/// Call [`sandbox_linux::try_run_helper`] in `main` before this so the Linux helper stays
/// single-threaded ahead of the Tokio runtime.
#[must_use]
pub fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().ok();
            return ExitCode::from(err.exit_code().clamp(0, 255) as u8);
        }
    };

    let Some(command) = cli.command else {
        return commands::wizard::run();
    };

    let launch_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resolved_config_env = match &command {
        Commands::Run(args) => resolve_agent_identity(
            &args.identity.project_root,
            &args.identity.agent,
            args.home.as_deref(),
        )
        .ok()
        .map(|p| resolve_config_path_for_dotenv(&p.config, &launch_cwd)),
        Commands::Acp(args) => {
            let root = args
                .project_root
                .clone()
                .unwrap_or_else(|| launch_cwd.clone());
            resolve_agent_identity(&root, &args.agent, args.home.as_deref())
                .ok()
                .map(|p| resolve_config_path_for_dotenv(&p.config, &launch_cwd))
        }
        Commands::A2a(args) => resolve_agent_identity(
            &args.identity.project_root,
            &args.identity.agent,
            args.home.as_deref(),
        )
        .ok()
        .map(|p| resolve_config_path_for_dotenv(&p.config, &launch_cwd)),
        Commands::Check(args) => {
            resolve_agent_identity(&args.identity.project_root, &args.identity.agent, None)
                .ok()
                .map(|p| resolve_config_path_for_dotenv(&p.config, &launch_cwd))
        }
        Commands::Config(args) => {
            resolve_agent_identity(&args.identity.project_root, &args.identity.agent, None)
                .ok()
                .map(|p| resolve_config_path_for_dotenv(&p.config, &launch_cwd))
        }
        Commands::Init(_) => None,
    };
    // Precedence: process env > config-dir .env > launch CWD .env
    load_dotenv_layered(resolved_config_env.as_deref());

    match command {
        Commands::Run(args) => run_worker(args),
        Commands::Acp(args) => run_acp(args),
        Commands::A2a(args) => run_a2a(args),
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
        Commands::Config(args) => match commands::config::run(&args) {
            Ok(()) => ExitCode::SUCCESS,
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

    let prompt_args = match AgentPromptArgs::try_from(args) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("error: {err:#}");
            return ExitCode::FAILURE;
        }
    };

    let output = match runtime.block_on(run_agent_prompt(prompt_args)) {
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
                "{{\"run_id\":\"serialization-error\",\"result\":\"failed to serialize output\",\"usage\":{{\"iterations\":0,\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0,\"elapsed_ms\":0,\"cost\":{{\"status\":\"unavailable\",\"known_total\":null,\"priced_requests\":0,\"unpriced_requests\":0,\"budget_status\":\"not_configured\",\"requests\":[]}}}},\"stop_reason\":\"error\",\"logs\":{{\"ai_log\":null,\"mcp_log\":null,\"system_log\":null,\"chat_log\":null}}}}"
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

fn run_a2a(args: A2aArgs) -> ExitCode {
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

    match runtime.block_on(a2a::run_a2a_server(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: A2A server failed: {err:#}");
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
        args.max_cost.clone(),
        args.save_chat_history,
    );
}

async fn build_billing_resolver(
    cfg: &AppConfig,
    config_path: &Path,
    billing_target: Option<&(String, String)>,
    billing_registry: Option<Arc<McpRegistry>>,
    logger: Option<SharedEventSink>,
) -> Result<Arc<CostResolverChain>> {
    let provider: Arc<dyn CostResolver> = Arc::new(
        ProviderReportedCostResolver::new(
            cfg.billing.currency.clone(),
            cfg.billing.provider_reported.unit.clone(),
        )
        .context("configuring provider-reported billing")?,
    );

    let catalog: Option<Arc<dyn CostResolver>> = if let Some(path) = &cfg.billing.catalog_path {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(path)
        };
        let catalog = tokio::task::spawn_blocking(move || PricingCatalog::load(&resolved))
            .await
            .context("loading pricing catalog task")?
            .context("loading pricing catalog")?;
        Some(Arc::new(
            CatalogCostResolver::new(catalog, cfg.billing.currency.clone())
                .context("configuring pricing catalog")?,
        ))
    } else {
        None
    };

    let mcp: Option<Arc<dyn CostResolver>> = match (billing_target, billing_registry) {
        (Some((server, tool)), Some(registry)) => {
            let qualified = format!("{server}.{tool}");
            if registry
                .available_tools()
                .iter()
                .any(|name| name == &qualified)
            {
                let timeout_ms = cfg
                    .billing
                    .mcp
                    .as_ref()
                    .map_or(5_000, |binding| binding.timeout_ms);
                Some(Arc::new(
                    McpCostResolver::new(
                        registry,
                        server.clone(),
                        tool.clone(),
                        cfg.billing.currency.clone(),
                        timeout_ms,
                        logger,
                    )
                    .context("configuring billing MCP resolver")?,
                ))
            } else {
                Some(Arc::new(UnavailableCostResolver::new(
                    "mcp",
                    format!("configured target `{qualified}` was not discovered"),
                )))
            }
        }
        (Some(_), None) => Some(Arc::new(UnavailableCostResolver::new(
            "mcp",
            "optional billing MCP failed to connect",
        ))),
        (None, _) => None,
    };

    let mut ordered: Vec<Arc<dyn CostResolver>> = Vec::new();
    for source in &cfg.billing.source_order {
        match source {
            BillingSource::ProviderReported => ordered.push(provider.clone()),
            BillingSource::Mcp => {
                if let Some(mcp) = &mcp {
                    ordered.push(mcp.clone());
                }
            }
            BillingSource::Catalog => {
                if let Some(catalog) = &catalog {
                    ordered.push(catalog.clone());
                }
            }
        }
    }
    Ok(Arc::new(CostResolverChain::new(ordered)))
}

fn generate_run_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("run-{timestamp:032x}-{:016x}", rand::random::<u64>())
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
    ConfigSafetyValidator::check(&cfg).context("config safety validation")?;
    if args.prompt.trim().is_empty() {
        bail!("prompt must not be empty");
    }
    let run_id = args.run_id.clone().unwrap_or_else(generate_run_id);
    if run_id.trim().is_empty() || run_id.len() > 128 {
        bail!("run_id must contain 1..=128 characters");
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

    let config_dir = args.config.parent().map(Path::to_path_buf);
    let workspace_root = workspace_root_for_run(
        &access_policy,
        &std::env::current_dir().context("resolving current working directory")?,
        args.project_root.as_deref(),
        config_dir.as_deref(),
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

    let billing_target = cfg
        .billing
        .mcp
        .as_ref()
        .and_then(|binding| binding.target.split_once('.'))
        .map(|(server, tool)| (server.to_string(), tool.to_string()));
    let regular_mcp_configs: Vec<_> = cfg
        .mcp
        .iter()
        .filter(|server| {
            billing_target
                .as_ref()
                .is_none_or(|(billing_server, _)| server.name != *billing_server)
        })
        .cloned()
        .collect();
    let mcp = Arc::new(
        McpRegistry::connect_all_isolated(
            &regular_mcp_configs,
            loggers.mcp.clone(),
            run_cancel.token().clone(),
            crate::mcp::McpIsolationContext {
                project_root: args.project_root.clone(),
                agent_id: args.agent_id.clone(),
            },
        )
        .await
        .context("connecting MCP servers")?,
    );
    let mcp_tools = parse_tool_list(mcp.available_tools())
        .map_err(|reason| anyhow::anyhow!("parsing MCP tool names: {reason}"))?;
    let effective = EffectiveToolPolicy::compile(&access_policy, &skills_allowed, mcp_tools);
    let pipeline_events = EventMcpDispatcher::new(&cfg.event_mcp, mcp.clone(), loggers.mcp.clone())
        .context("compiling Event-MCP handlers")?;

    let billing_registry = if let Some((server_name, _)) = &billing_target {
        let server_config = cfg
            .mcp
            .iter()
            .find(|server| server.name == *server_name)
            .expect("billing MCP target validated");
        match McpRegistry::connect_all_isolated(
            std::slice::from_ref(server_config),
            loggers.mcp.clone(),
            run_cancel.token().clone(),
            crate::mcp::McpIsolationContext {
                project_root: args.project_root.clone(),
                agent_id: args.agent_id.clone(),
            },
        )
        .await
        {
            Ok(registry) => Some(Arc::new(registry)),
            Err(error) => {
                warn!(
                    server = server_name,
                    error = %error,
                    "optional billing MCP unavailable; continuing with fallback sources"
                );
                None
            }
        }
    } else {
        None
    };
    let billing_resolver = build_billing_resolver(
        &cfg,
        &args.config,
        billing_target.as_ref(),
        billing_registry,
        loggers.mcp.clone(),
    )
    .await?;

    let provider = OpenAiCompatClient::new_with_accounting(
        cfg.provider.clone(),
        ProviderAccountingOptions {
            provider_id: cfg.billing.provider_id.clone(),
            reported_cost_unit: cfg.billing.provider_reported.unit.clone(),
            cost_json_pointers: cfg.billing.provider_reported.json_pointers.clone(),
            cost_headers: cfg.billing.provider_reported.headers.clone(),
        },
    )
    .context("initializing provider")?;
    let tools = PolicyToolExecutor::new(
        Arc::new(CompositeToolExecutor::new(home, local_tools, mcp)),
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

    let engine = AgentEngine::new(Arc::new(provider), Arc::new(tools), loggers)
        .with_pipeline_events(Arc::new(pipeline_events))
        .with_billing(billing_resolver, cfg.billing.currency.clone(), run_id);
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
