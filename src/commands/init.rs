//! Scaffold a new agent profile under `.kuibysheff/protected/agents/<id>/`.

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::access::{ensure_protected_profile_dirs, AccessPolicyConfig};
use crate::cli::InitArgs;
use crate::commands::prompt::{self, PromptError};
use crate::project_paths::{
    agent_profile_dir, validate_agent_id, AgentPathError, AGENT_CONFIG_FILE,
};

const MASTER_PROMPT: &str = include_str!("../templates/agent_init/master_prompt.md");
const SKILLS_DSL: &str = include_str!("../templates/agent_init/skills.dsl");
const RULES_MD: &str = include_str!("../templates/agent_init/rules.md");
const AGENT_CONFIG: &str = include_str!("../templates/agent_init/agent-config.example.yaml");

const CONFIG_ACCESS_FOOTER: &str = r#"
# Required capability policy (fail-closed). For permissive FS only:
#   access:
#     mode: legacy
"#;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InitError {
    #[error(transparent)]
    AgentPath(#[from] AgentPathError),
    #[error("target directory `{0}` already exists and is not empty; pass `--force` to overwrite template files")]
    TargetNotEmpty(String),
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
    #[error("`--interactive` requires a terminal (stdin/stdout)")]
    InteractiveRequiresTty,
    #[error(transparent)]
    Prompt(#[from] PromptError),
    #[error(transparent)]
    Dotenv(#[from] crate::commands::dotenv_file::DotenvFileError),
}

/// Values collected for the starter runtime config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigAnswers {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    /// Optional secret to persist in profile `.env` (never written into YAML).
    pub api_key: Option<String>,
    pub max_iterations: u32,
    pub max_tokens: u64,
    pub max_duration_sec: u64,
}

impl Default for ConfigAnswers {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            api_key: None,
            max_iterations: 10,
            max_tokens: 15_000,
            max_duration_sec: 120,
        }
    }
}

/// Result of a successful `init` scaffold.
#[derive(Debug, Clone)]
pub struct InitResult {
    pub agent_id: String,
    pub project_root: PathBuf,
    pub target_dir: PathBuf,
    pub written_files: Vec<PathBuf>,
}

/// Validate `agent-id` and create the protected profile scaffold.
///
/// # Errors
///
/// Returns [`InitError`] when the id is invalid, the target exists without
/// `--force`, prompting fails, or filesystem operations fail.
pub fn run(args: &InitArgs) -> Result<InitResult, InitError> {
    validate_agent_id(&args.agent_id)?;
    let target_dir = agent_profile_dir(&args.project_root, &args.agent_id)?;

    ensure_protected_profile_dirs(&target_dir).map_err(|source| InitError::CreateDir {
        path: target_dir.display().to_string(),
        source,
    })?;
    prepare_target_dir(&target_dir, args.force)?;

    let config_body = if args.interactive {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(InitError::InteractiveRequiresTty);
        }
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout();
        let answers = prompt_config(&mut stdin, &mut stdout)?;
        let body = render_agent_config(&answers);
        // Defer .env write until after profile dir files exist (below).
        let api_key = answers.api_key.clone();
        let api_key_env = answers.api_key_env.clone();
        let files = [
            ("master_prompt.md", MASTER_PROMPT.to_string()),
            ("skills.dsl", SKILLS_DSL.to_string()),
            ("rules.md", RULES_MD.to_string()),
            (AGENT_CONFIG_FILE, body),
        ];
        let mut written_files = Vec::with_capacity(files.len());
        for (name, contents) in files {
            let path = target_dir.join(name);
            fs::write(&path, contents).map_err(|source| InitError::WriteFile {
                path: path.display().to_string(),
                source,
            })?;
            written_files.push(path);
        }
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            let env_path = target_dir.join(".env");
            crate::commands::dotenv_file::upsert_env_var(&env_path, &api_key_env, &key)?;
            written_files.push(env_path);
        }
        return Ok(InitResult {
            agent_id: args.agent_id.clone(),
            project_root: args.project_root.clone(),
            target_dir,
            written_files,
        });
    } else {
        AGENT_CONFIG.to_string()
    };

    let files = [
        ("master_prompt.md", MASTER_PROMPT.to_string()),
        ("skills.dsl", SKILLS_DSL.to_string()),
        ("rules.md", RULES_MD.to_string()),
        (AGENT_CONFIG_FILE, config_body),
    ];

    let mut written_files = Vec::with_capacity(files.len());
    for (name, contents) in files {
        let path = target_dir.join(name);
        fs::write(&path, contents).map_err(|source| InitError::WriteFile {
            path: path.display().to_string(),
            source,
        })?;
        written_files.push(path);
    }

    Ok(InitResult {
        agent_id: args.agent_id.clone(),
        project_root: args.project_root.clone(),
        target_dir,
        written_files,
    })
}

/// Print a human-readable success summary for `init`.
pub fn print_success(result: &InitResult) {
    println!(
        "Created agent `{}` at `{}`",
        result.agent_id,
        result.target_dir.display()
    );
    for path in &result.written_files {
        println!("  {}", path.display());
    }
    println!();
    println!("Example run:");
    println!(
        "  kbshff run \\\n    --project-root {} \\\n    --agent {} \\\n    --prompt \"...\"",
        result.project_root.display(),
        result.agent_id
    );
    println!();
    println!("Import external settings:");
    println!(
        "  kbshff config --project-root {} --agent {} import --from <PATH>",
        result.project_root.display(),
        result.agent_id
    );
}

/// Ask for provider and limits on `reader`/`writer`. Empty input keeps the default.
///
/// Also asks for an optional API key value (stored later in profile `.env`, not YAML).
///
/// # Errors
///
/// Returns [`InitError::Prompt`] on I/O or parse failures.
pub fn prompt_config<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> Result<ConfigAnswers, InitError> {
    let defaults = ConfigAnswers::default();
    writeln!(
        writer,
        "Configure runtime settings (press Enter to keep the default)."
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
    let api_key = if api_key_raw.is_empty() {
        None
    } else {
        Some(api_key_raw)
    };
    let api_key_env = loop {
        let name = prompt::prompt_string(reader, writer, "API key env var", &defaults.api_key_env)?;
        match crate::commands::dotenv_file::validate_env_var_name(&name) {
            Ok(()) => break name,
            Err(err) => {
                writeln!(writer, "{err}").map_err(PromptError::io)?;
                writeln!(writer, "Please try again.").map_err(PromptError::io)?;
            }
        }
    };
    let max_iterations = prompt::prompt_parse(
        reader,
        writer,
        "Max iterations",
        defaults.max_iterations,
        |s| s.parse::<u32>(),
    )?;
    let max_tokens =
        prompt::prompt_parse(reader, writer, "Max tokens", defaults.max_tokens, |s| {
            s.parse::<u64>()
        })?;
    let max_duration_sec = prompt::prompt_parse(
        reader,
        writer,
        "Max duration (sec)",
        defaults.max_duration_sec,
        |s| s.parse::<u64>(),
    )?;

    Ok(ConfigAnswers {
        base_url,
        model,
        api_key_env,
        api_key,
        max_iterations,
        max_tokens,
        max_duration_sec,
    })
}

/// Render a starter `agent-config.yaml` from interactive answers.
#[must_use]
pub fn render_agent_config(answers: &ConfigAnswers) -> String {
    let mut body = format!(
        r#"provider:
  base_url: {base_url}
  model: {model}
  api_key_env: {api_key_env}
  timeout_ms: 60000
  max_retries: 3
  retry_base_delay_ms: 500
  history:
    max_tail_messages: 30
    max_chars: 200000

mcp: []

billing:
  provider_id: "openai"
  currency: "USD"
  source_order: ["provider_reported", "mcp", "catalog"]
  provider_reported:
    unit: "USD"
    json_pointers: ["/usage/cost", "/usage/response_cost/total_cost"]
    headers: ["x-litellm-response-cost"]
  on_unpriced: continue

limits:
  max_iterations: {max_iterations}
  max_tokens: {max_tokens}
  max_duration_sec: {max_duration_sec}

logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: false
  sink:
    type: file
"#,
        base_url = yaml_string(&answers.base_url),
        model = yaml_string(&answers.model),
        api_key_env = yaml_string(&answers.api_key_env),
        max_iterations = answers.max_iterations,
        max_tokens = answers.max_tokens,
        max_duration_sec = answers.max_duration_sec,
    );
    body.push_str(CONFIG_ACCESS_FOOTER);
    #[derive(serde::Serialize)]
    struct AccessOnly {
        access: AccessPolicyConfig,
    }
    match serde_yaml::to_string(&AccessOnly {
        access: AccessPolicyConfig::minimal_profile(),
    }) {
        Ok(yaml) => body.push_str(&yaml),
        Err(_) => body.push_str(
            "access:\n  tools:\n    builtins:\n      - home.list\n      - home.read\n      - home.write\n  filesystem:\n    home:\n      read: [\"in\", \"out\"]\n      write: [\"out\"]\n",
        ),
    }
    body
}

fn yaml_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn prepare_target_dir(target_dir: &Path, force: bool) -> Result<(), InitError> {
    match fs::metadata(target_dir) {
        Ok(meta) if meta.is_dir() => {
            let is_empty = fs::read_dir(target_dir)
                .map_err(|source| InitError::CreateDir {
                    path: target_dir.display().to_string(),
                    source,
                })?
                .next()
                .is_none();
            if !is_empty && !force {
                return Err(InitError::TargetNotEmpty(target_dir.display().to_string()));
            }
            Ok(())
        }
        Ok(_) => Err(InitError::CreateDir {
            path: target_dir.display().to_string(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "path exists and is not a directory",
            ),
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => fs::create_dir_all(target_dir)
            .map_err(|source| InitError::CreateDir {
                path: target_dir.display().to_string(),
                source,
            }),
        Err(source) => Err(InitError::CreateDir {
            path: target_dir.display().to_string(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::settings::load_settings;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn args(project: &Path, agent_id: &str, force: bool) -> InitArgs {
        InitArgs {
            agent_id: agent_id.to_string(),
            project_root: project.to_path_buf(),
            force,
            interactive: false,
        }
    }

    #[test]
    fn rejects_invalid_agent_id() {
        for id in ["", "has/slash", "has..dots", "-leading", "a:b", " trailing"] {
            let err = validate_agent_id(id).expect_err("should reject");
            assert!(
                matches!(err, AgentPathError::InvalidAgentId(_)),
                "id={id:?} err={err:?}"
            );
        }
        for id in ["Mulder", "Agent Mulder", "demo", "агент-1"] {
            validate_agent_id(id).unwrap_or_else(|e| panic!("should accept {id:?}: {e}"));
        }
    }

    #[test]
    fn scaffolds_protected_profile() {
        let dir = tempdir().expect("tempdir");
        let result = run(&args(dir.path(), "demo", false)).expect("init");

        assert_eq!(result.written_files.len(), 4);
        let settings = load_settings(&result.target_dir).expect("load settings");
        assert!(!settings.master_prompt.is_empty());
        assert!(!settings.skills_source.is_empty());
        assert!(result.target_dir.join(AGENT_CONFIG_FILE).is_file());
        let _ = load_config(&result.target_dir.join(AGENT_CONFIG_FILE)).expect("load cfg");
    }

    #[test]
    fn refuses_non_empty_without_force() {
        let dir = tempdir().expect("tempdir");
        run(&args(dir.path(), "demo", false)).expect("first init");
        let err = run(&args(dir.path(), "demo", false)).expect_err("second init");
        assert!(matches!(err, InitError::TargetNotEmpty(_)));
    }

    #[test]
    fn force_overwrites() {
        let dir = tempdir().expect("tempdir");
        run(&args(dir.path(), "demo", false)).expect("first");
        run(&args(dir.path(), "demo", true)).expect("force");
    }

    #[test]
    fn interactive_render_parses() {
        let yaml = render_agent_config(&ConfigAnswers::default());
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("c.yaml");
        fs::write(&path, &yaml).unwrap();
        let _ = load_config(&path).expect("parse rendered");
    }

    #[test]
    fn prompt_config_keeps_defaults_on_empty() {
        // base_url, model, api_key, api_key_env, max_iterations, max_tokens, max_duration_sec
        let mut input = Cursor::new("\n\n\n\n\n\n\n");
        let mut out = Vec::new();
        let answers = prompt_config(&mut input, &mut out).expect("prompt");
        assert_eq!(answers, ConfigAnswers::default());
    }
}
