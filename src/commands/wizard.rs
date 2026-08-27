//! Interactive setup wizard when the CLI is launched without a subcommand.

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::CommandFactory;
use thiserror::Error;

use crate::cli::{
    AgentIdentityArgs, CheckArgs, ConfigArgs, ConfigCommand, ConfigFormat, InitArgs, LimitsCmd,
    McpCmd, ProviderCmd,
};
use crate::commands::dotenv_file::{self, DotenvFileError};
use crate::commands::prompt::PromptError;
use crate::commands::{check, config, init, prompt};
use crate::config::load_dotenv_layered;
use crate::project_paths::{list_agent_ids, validate_agent_id, AGENT_CONFIG_FILE};

/// Fatal wizard errors.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WizardError {
    #[error(transparent)]
    Prompt(#[from] PromptError),
    #[error("failed to create directory `{path}`: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to write `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to list agents: {0}")]
    ListAgents(#[source] io::Error),
    #[error(transparent)]
    Init(#[from] init::InitError),
    #[error(transparent)]
    Check(#[from] check::CheckError),
    #[error(transparent)]
    Config(#[from] config::ConfigCmdError),
    #[error(transparent)]
    Dotenv(#[from] DotenvFileError),
    #[error("{0}")]
    Message(String),
}

/// Entry for `kbshff` with no subcommand.
#[must_use]
pub fn run() -> ExitCode {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        // Non-interactive: print help via clap by re-parsing with --help.
        let mut cmd = crate::cli::Cli::command();
        let _ = cmd.print_help();
        eprintln!();
        eprintln!("error: interactive setup requires a terminal; pass a subcommand (run, init, check, config, acp, a2a)");
        return ExitCode::from(2);
    }

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout();
    match run_with(&mut stdin, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Testable wizard body.
///
/// # Errors
///
/// Returns [`WizardError`] on prompt, filesystem, or config failures.
pub fn run_with<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> Result<(), WizardError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd_display = cwd.display().to_string();

    writeln!(writer, "kbshff interactive setup").map_err(PromptError::io)?;
    writeln!(writer).map_err(PromptError::io)?;

    let path_raw = prompt::prompt_string(reader, writer, "Harness folder", &cwd_display)?;
    let project_root = resolve_harness_path(&cwd, &path_raw);

    if !project_root.exists() {
        let create = prompt::prompt_yes_no(
            reader,
            writer,
            &format!(
                "Directory `{}` does not exist. Create it?",
                project_root.display()
            ),
            true,
        )?;
        if !create {
            writeln!(writer, "Aborted.").map_err(PromptError::io)?;
            return Ok(());
        }
        fs::create_dir_all(&project_root).map_err(|source| WizardError::CreateDir {
            path: project_root.display().to_string(),
            source,
        })?;
        writeln!(writer, "Created `{}`.", project_root.display()).map_err(PromptError::io)?;
        let agent_id = scaffold_new_agent(reader, writer, &project_root)?;
        return continue_after_profile(reader, writer, &project_root, &agent_id);
    }

    if !project_root.is_dir() {
        return Err(WizardError::Message(format!(
            "`{}` exists and is not a directory",
            project_root.display()
        )));
    }

    let agents = list_agent_ids(&project_root).map_err(WizardError::ListAgents)?;
    let agent_id = match agents.as_slice() {
        [] => {
            writeln!(
                writer,
                "No agent profiles found under `.kuibysheff/protected/agents/`."
            )
            .map_err(PromptError::io)?;
            scaffold_new_agent(reader, writer, &project_root)?
        }
        [only] => {
            writeln!(writer, "Using agent `{only}`.").map_err(PromptError::io)?;
            only.clone()
        }
        many => select_agent(reader, writer, many)?,
    };

    continue_after_profile(reader, writer, &project_root, &agent_id)
}

fn resolve_harness_path(cwd: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn scaffold_new_agent<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    project_root: &Path,
) -> Result<String, WizardError> {
    let agent_id = loop {
        let candidate = prompt::prompt_string(reader, writer, "New agent id", "demo")?;
        match validate_agent_id(&candidate) {
            Ok(()) => break candidate,
            Err(err) => {
                writeln!(writer, "{err}").map_err(PromptError::io)?;
                writeln!(writer, "Please try again.").map_err(PromptError::io)?;
            }
        }
    };
    let answers = init::prompt_config(reader, writer)?;
    let result = init::run(&InitArgs {
        agent_id: agent_id.clone(),
        project_root: project_root.to_path_buf(),
        force: false,
        interactive: false,
    })?;
    let config_path = result.target_dir.join(AGENT_CONFIG_FILE);
    fs::write(&config_path, init::render_agent_config(&answers)).map_err(|source| {
        WizardError::WriteFile {
            path: config_path.display().to_string(),
            source,
        }
    })?;
    if let Some(key) = answers.api_key.as_ref().filter(|k| !k.is_empty()) {
        let env_path = result.target_dir.join(".env");
        dotenv_file::upsert_env_var(&env_path, &answers.api_key_env, key)?;
        writeln!(
            writer,
            "Wrote API key to `{}` (gitignored `.env`).",
            env_path.display()
        )
        .map_err(PromptError::io)?;
    }
    init::print_success(&result);
    Ok(agent_id)
}

fn select_agent<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    agents: &[String],
) -> Result<String, WizardError> {
    writeln!(writer, "Available agents:").map_err(PromptError::io)?;
    for (idx, id) in agents.iter().enumerate() {
        writeln!(writer, "  {}. {id}", idx + 1).map_err(PromptError::io)?;
    }
    let default = "1".to_string();
    loop {
        let choice = prompt::prompt_string(reader, writer, "Select agent number", &default)?;
        if let Ok(n) = choice.parse::<usize>() {
            if let Some(id) = agents.get(n.wrapping_sub(1)) {
                return Ok(id.clone());
            }
        }
        if agents.iter().any(|a| a == &choice) {
            return Ok(choice);
        }
        writeln!(writer, "Invalid selection; try again.").map_err(PromptError::io)?;
    }
}

fn continue_after_profile<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    project_root: &Path,
    agent_id: &str,
) -> Result<(), WizardError> {
    run_check_and_inventory(writer, project_root, agent_id)?;
    config_menu(reader, writer, project_root, agent_id)
}

fn run_check_and_inventory<W: Write>(
    writer: &mut W,
    project_root: &Path,
    agent_id: &str,
) -> Result<(), WizardError> {
    writeln!(writer).map_err(PromptError::io)?;
    let paths = crate::project_paths::resolve_agent_identity(project_root, agent_id, None)
        .map_err(|e| WizardError::Message(e.to_string()))?;
    load_dotenv_layered(Some(&paths.config));
    let report = check::run(&CheckArgs {
        identity: identity(project_root, agent_id),
        skip_provider: false,
        skip_mcp: false,
        skip_sandbox: false,
    })?;
    check::print_report(&report);
    check::print_inventory(&report.inventory);
    let _ = writer;
    Ok(())
}

fn identity(project_root: &Path, agent_id: &str) -> AgentIdentityArgs {
    AgentIdentityArgs {
        project_root: project_root.to_path_buf(),
        agent: agent_id.to_string(),
    }
}

fn config_args(project_root: &Path, agent_id: &str, command: ConfigCommand) -> ConfigArgs {
    ConfigArgs {
        identity: identity(project_root, agent_id),
        format: ConfigFormat::Text,
        command,
    }
}

fn config_menu<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    project_root: &Path,
    agent_id: &str,
) -> Result<(), WizardError> {
    loop {
        writeln!(writer).map_err(PromptError::io)?;
        writeln!(writer, "Continue configuring agent `{agent_id}`?").map_err(PromptError::io)?;
        writeln!(writer, "  1. Add MCP server").map_err(PromptError::io)?;
        writeln!(writer, "  2. Change provider").map_err(PromptError::io)?;
        writeln!(writer, "  3. Change limits").map_err(PromptError::io)?;
        writeln!(writer, "  4. Show profile").map_err(PromptError::io)?;
        writeln!(writer, "  5. Re-run check").map_err(PromptError::io)?;
        writeln!(writer, "  6. Quit").map_err(PromptError::io)?;

        let choice = prompt::prompt_string(reader, writer, "Choice", "6")?;
        match choice.as_str() {
            "1" => add_mcp_interactive(reader, writer, project_root, agent_id)?,
            "2" => change_provider_interactive(reader, writer, project_root, agent_id)?,
            "3" => change_limits_interactive(reader, writer, project_root, agent_id)?,
            "4" => {
                config::run(&config_args(project_root, agent_id, ConfigCommand::Show))?;
            }
            "5" => run_check_and_inventory(writer, project_root, agent_id)?,
            "6" | "q" | "quit" => {
                writeln!(writer, "Done.").map_err(PromptError::io)?;
                return Ok(());
            }
            _ => writeln!(writer, "Unknown choice.").map_err(PromptError::io)?,
        }
    }
}

fn add_mcp_interactive<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    project_root: &Path,
    agent_id: &str,
) -> Result<(), WizardError> {
    let name = prompt::prompt_required(reader, writer, "MCP server name")?;
    let transport = prompt::prompt_string(reader, writer, "Transport (stdio/http)", "stdio")?;
    match transport.to_ascii_lowercase().as_str() {
        "stdio" => {
            let command = prompt::prompt_required(reader, writer, "Command")?;
            let args_line = prompt::prompt_string(reader, writer, "Args (space-separated)", "")?;
            let args: Vec<String> = if args_line.is_empty() {
                Vec::new()
            } else {
                args_line.split_whitespace().map(str::to_string).collect()
            };
            config::run(&config_args(
                project_root,
                agent_id,
                ConfigCommand::Mcp(McpCmd::Add {
                    name,
                    command: Some(command),
                    args,
                    url: None,
                    env: Vec::new(),
                    headers: Vec::new(),
                    cwd: None,
                    timeout_ms: None,
                }),
            ))?;
        }
        "http" => {
            let url = prompt::prompt_required(reader, writer, "URL")?;
            config::run(&config_args(
                project_root,
                agent_id,
                ConfigCommand::Mcp(McpCmd::Add {
                    name,
                    command: None,
                    args: Vec::new(),
                    url: Some(url),
                    env: Vec::new(),
                    headers: Vec::new(),
                    cwd: None,
                    timeout_ms: None,
                }),
            ))?;
        }
        other => {
            return Err(WizardError::Message(format!(
                "unknown transport `{other}` (expected stdio or http)"
            )));
        }
    }
    Ok(())
}

fn change_provider_interactive<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    project_root: &Path,
    agent_id: &str,
) -> Result<(), WizardError> {
    let defaults = init::ConfigAnswers::default();
    writeln!(
        writer,
        "Configure provider (press Enter to keep the default)."
    )
    .map_err(PromptError::io)?;
    let base_url = prompt::prompt_string(reader, writer, "Provider base URL", &defaults.base_url)?;
    let model = prompt::prompt_string(reader, writer, "Model", &defaults.model)?;
    writeln!(
        writer,
        "API key is stored in the agent profile `.env` (not in YAML). Leave empty to skip."
    )
    .map_err(PromptError::io)?;
    let api_key_raw = prompt::prompt_string(reader, writer, "API key", "")?;
    let api_key_env = loop {
        let name = prompt::prompt_string(reader, writer, "API key env var", &defaults.api_key_env)?;
        match dotenv_file::validate_env_var_name(&name) {
            Ok(()) => break name,
            Err(err) => {
                writeln!(writer, "{err}").map_err(PromptError::io)?;
                writeln!(writer, "Please try again.").map_err(PromptError::io)?;
            }
        }
    };
    config::run(&config_args(
        project_root,
        agent_id,
        ConfigCommand::Provider(ProviderCmd::Set {
            base_url: Some(base_url),
            model: Some(model),
            api_key_env: Some(api_key_env.clone()),
            timeout_ms: None,
            max_retries: None,
        }),
    ))?;
    if !api_key_raw.is_empty() {
        let paths = crate::project_paths::resolve_agent_identity(project_root, agent_id, None)
            .map_err(|e| WizardError::Message(e.to_string()))?;
        let env_path = paths.profile_dir.join(".env");
        dotenv_file::upsert_env_var(&env_path, &api_key_env, &api_key_raw)?;
        load_dotenv_layered(Some(&paths.config));
        writeln!(writer, "Wrote API key to `{}`.", env_path.display()).map_err(PromptError::io)?;
    }
    Ok(())
}

fn change_limits_interactive<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    project_root: &Path,
    agent_id: &str,
) -> Result<(), WizardError> {
    let max_iterations = prompt::prompt_parse(reader, writer, "Max iterations", 10u32, |s| {
        s.parse::<u32>()
    })?;
    let max_tokens = prompt::prompt_parse(reader, writer, "Max tokens", 15_000u64, |s| {
        s.parse::<u64>()
    })?;
    let max_duration_sec =
        prompt::prompt_parse(reader, writer, "Max duration (sec)", 120u64, |s| {
            s.parse::<u64>()
        })?;
    config::run(&config_args(
        project_root,
        agent_id,
        ConfigCommand::Limits(LimitsCmd::Set {
            max_iterations: Some(max_iterations),
            max_tokens: Some(max_tokens),
            max_duration_sec: Some(max_duration_sec),
            max_cost: None,
        }),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn aborts_when_user_declines_create() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist-yet");
        let input = format!("{}\nn\n", missing.display());
        let mut reader = Cursor::new(input);
        let mut out = Vec::new();
        run_with(&mut reader, &mut out).expect("wizard");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("Aborted"), "{text}");
        assert!(!missing.exists());
    }

    #[test]
    fn uses_existing_single_agent_then_quits() {
        let dir = tempfile::tempdir().unwrap();
        init::run(&InitArgs {
            agent_id: "demo".to_string(),
            project_root: dir.path().to_path_buf(),
            force: false,
            interactive: false,
        })
        .expect("init");

        // path default via empty → but we pass absolute path; then quit menu
        let input = format!("{}\n6\n", dir.path().display());
        let mut reader = Cursor::new(input);
        let mut out = Vec::new();
        // check will run (may fail provider) — still Ok for wizard
        let result = run_with(&mut reader, &mut out);
        assert!(result.is_ok(), "{result:?}");
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("Using agent `demo`") || text.contains("Done"),
            "{text}"
        );
    }

    #[test]
    fn resolve_harness_joins_relative() {
        let cwd = Path::new("/base");
        assert_eq!(
            resolve_harness_path(cwd, "proj"),
            PathBuf::from("/base/proj")
        );
    }

    #[test]
    fn creates_missing_dir_and_agent_then_quits() {
        let parent = tempfile::tempdir().unwrap();
        let harness = parent.path().join("new-harness");
        // path, yes, agent id default, then 7 config prompts (incl. empty API key), quit
        let input = format!("{}\ny\n\n\n\n\n\n\n\n6\n", harness.display());
        let mut reader = Cursor::new(input);
        let mut out = Vec::new();
        run_with(&mut reader, &mut out).expect("wizard");
        assert!(harness.is_dir());
        let agents = list_agent_ids(&harness).expect("list");
        assert_eq!(agents, vec!["demo".to_string()]);
    }

    #[test]
    fn writes_api_key_to_profile_dotenv() {
        let dir = tempfile::tempdir().unwrap();
        // path, agent demo (empty), base_url, model, api_key, env var, 3 limits, quit
        let input = format!("{}\n\n\n\nsk-test-key\n\n\n\n\n6\n", dir.path().display());
        let mut reader = Cursor::new(input);
        let mut out = Vec::new();
        run_with(&mut reader, &mut out).expect("wizard");
        let env_path = dir
            .path()
            .join(".kuibysheff")
            .join("protected")
            .join("agents")
            .join("demo")
            .join(".env");
        let text = fs::read_to_string(&env_path).expect(".env");
        assert!(text.contains("OPENAI_API_KEY=sk-test-key"), "env={text}");
    }

    #[test]
    fn retries_invalid_agent_id_then_accepts_unicode_name() {
        let dir = tempfile::tempdir().unwrap();
        // existing empty harness → create agent; first id invalid, second ok
        let input = format!("{}\n-bad\nMulder\n\n\n\n\n\n\n\n6\n", dir.path().display());
        let mut reader = Cursor::new(input);
        let mut out = Vec::new();
        run_with(&mut reader, &mut out).expect("wizard");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("Please try again."), "{text}");
        assert_eq!(
            list_agent_ids(dir.path()).expect("list"),
            vec!["Mulder".to_string()]
        );
    }
}
