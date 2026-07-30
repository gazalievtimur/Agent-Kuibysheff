//! Scaffold a new agent settings directory and starter config.

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::cli::InitArgs;

const MASTER_PROMPT: &str = include_str!("../templates/agent_init/master_prompt.md");
const SKILLS_DSL: &str = include_str!("../templates/agent_init/skills.dsl");
const RULES_MD: &str = include_str!("../templates/agent_init/rules.md");
const AGENT_CONFIG: &str = include_str!("../templates/agent_init/agent-config.example.yaml");

const CONFIG_ACCESS_FOOTER: &str = r#"
# Optional fail-closed capability policy.
# Omit `access` for legacy filesystem mode.
# When present, everything not listed is denied.
#
# access:
#   tools:
#     builtins:
#       - home.list
#       - home.read
#       - home.write
#   filesystem:
#     home:
#       read: ["in", "out"]
#       write: ["out"]
"#;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InitError {
    #[error("invalid agent id `{0}`: use `[a-z0-9][a-z0-9_-]*` without path separators")]
    InvalidAgentId(String),
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
    #[error("interactive prompt failed: {0}")]
    Prompt(String),
}

/// Values collected for the starter runtime config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigAnswers {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
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
    pub target_dir: PathBuf,
    pub written_files: Vec<PathBuf>,
}

/// Validate `agent-id` and create the settings + starter config scaffold.
///
/// # Errors
///
/// Returns [`InitError`] when the id is invalid, the target exists without
/// `--force`, prompting fails, or filesystem operations fail.
pub fn run(args: &InitArgs) -> Result<InitResult, InitError> {
    validate_agent_id(&args.agent_id)?;

    let target_dir = args
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from(&args.agent_id));

    prepare_target_dir(&target_dir, args.force)?;

    let config_body = if args.interactive {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(InitError::InteractiveRequiresTty);
        }
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout();
        let answers = prompt_config(&mut stdin, &mut stdout)?;
        render_agent_config(&answers)
    } else {
        AGENT_CONFIG.to_string()
    };

    let files = [
        ("master_prompt.md", MASTER_PROMPT.to_string()),
        ("skills.dsl", SKILLS_DSL.to_string()),
        ("rules.md", RULES_MD.to_string()),
        ("agent-config.example.yaml", config_body),
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
    let config = result.target_dir.join("agent-config.example.yaml");
    println!();
    println!("Example run:");
    println!(
        "  agent_Kuibyshev run \\\n    --config {} \\\n    --settings-dir {} \\\n    --prompt \"...\" \\\n    --home ./demo-home/{}",
        config.display(),
        result.target_dir.display(),
        result.agent_id
    );
}

/// Ask for provider and limits on `reader`/`writer`. Empty input keeps the default.
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
    .map_err(|e| InitError::Prompt(e.to_string()))?;

    let base_url = prompt_string(reader, writer, "Provider base URL", &defaults.base_url)?;
    let model = prompt_string(reader, writer, "Model", &defaults.model)?;
    let api_key_env = prompt_string(reader, writer, "API key env var", &defaults.api_key_env)?;
    let max_iterations = prompt_parse(
        reader,
        writer,
        "Max iterations",
        defaults.max_iterations,
        |s| s.parse::<u32>(),
    )?;
    let max_tokens = prompt_parse(reader, writer, "Max tokens", defaults.max_tokens, |s| {
        s.parse::<u64>()
    })?;
    let max_duration_sec = prompt_parse(
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
        max_iterations,
        max_tokens,
        max_duration_sec,
    })
}

/// Render a starter `agent-config.example.yaml` from interactive answers.
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
  # Context-window pruning for this model (independent of limits.max_tokens).
  history:
    max_tail_messages: 30
    max_chars: 200000

mcp: []
# Add MCP servers with a future `add-mcp` command, or edit this list by hand.

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
    body
}

fn yaml_string(value: &str) -> String {
    // Always quote so user input cannot break YAML structure.
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn prompt_string<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: &str,
) -> Result<String, InitError> {
    write!(writer, "{label} [{default}]: ").map_err(|e| InitError::Prompt(e.to_string()))?;
    writer
        .flush()
        .map_err(|e| InitError::Prompt(e.to_string()))?;
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| InitError::Prompt(e.to_string()))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_parse<R, W, T, E, F>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: T,
    parse: F,
) -> Result<T, InitError>
where
    R: BufRead,
    W: Write,
    T: ToString + Copy,
    E: std::fmt::Display,
    F: Fn(&str) -> Result<T, E>,
{
    let default_text = default.to_string();
    let raw = prompt_string(reader, writer, label, &default_text)?;
    if raw == default_text {
        return Ok(default);
    }
    parse(&raw).map_err(|e| InitError::Prompt(format!("invalid {label}: {e}")))
}

fn validate_agent_id(agent_id: &str) -> Result<(), InitError> {
    let mut chars = agent_id.chars();
    let Some(first) = chars.next() else {
        return Err(InitError::InvalidAgentId(agent_id.to_string()));
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(InitError::InvalidAgentId(agent_id.to_string()));
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err(InitError::InvalidAgentId(agent_id.to_string()));
    }
    if agent_id.contains('/') || agent_id.contains('\\') || agent_id.contains("..") {
        return Err(InitError::InvalidAgentId(agent_id.to_string()));
    }
    Ok(())
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

    fn args(agent_id: &str, path: PathBuf, force: bool) -> InitArgs {
        InitArgs {
            agent_id: agent_id.to_string(),
            path: Some(path),
            force,
            interactive: false,
        }
    }

    #[test]
    fn rejects_invalid_agent_id() {
        for id in ["", "Bad", "has/slash", "has..dots", "UPPER", "-leading"] {
            let err = validate_agent_id(id).expect_err("should reject");
            assert!(
                matches!(err, InitError::InvalidAgentId(_)),
                "id={id:?} err={err:?}"
            );
        }
    }

    #[test]
    fn accepts_valid_agent_id() {
        for id in ["a", "agent1", "my-agent", "my_agent", "0x"] {
            validate_agent_id(id).expect("should accept");
        }
    }

    #[test]
    fn scaffolds_settings_and_config() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("demo");
        let result = run(&args("demo", target.clone(), false)).expect("init");

        assert_eq!(result.written_files.len(), 4);
        let settings = load_settings(&target).expect("load settings");
        assert!(!settings.master_prompt.is_empty());
        assert!(!settings.skills_source.is_empty());
        assert!(!settings.rules.is_empty());
        assert!(target.join("agent-config.example.yaml").is_file());
    }

    #[test]
    fn refuses_non_empty_without_force() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("demo");
        run(&args("demo", target.clone(), false)).expect("first init");
        let err = run(&args("demo", target, false)).expect_err("second init");
        assert!(matches!(err, InitError::TargetNotEmpty(_)));
    }

    #[test]
    fn force_overwrites_template_files() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("demo");
        run(&args("demo", target.clone(), false)).expect("first init");
        fs::write(target.join("master_prompt.md"), "stale").expect("mutate");
        run(&args("demo", target.clone(), true)).expect("force init");
        let contents = fs::read_to_string(target.join("master_prompt.md")).expect("read");
        assert!(contents.contains("agent_Kuibyshev"));
        assert!(!contents.contains("stale"));
    }

    #[test]
    fn prompt_config_keeps_defaults_on_empty_lines() {
        let input = "\n\n\n\n\n\n";
        let mut reader = Cursor::new(input);
        let mut writer = Vec::new();
        let answers = prompt_config(&mut reader, &mut writer).expect("prompt");
        assert_eq!(answers, ConfigAnswers::default());
    }

    #[test]
    fn prompt_config_reads_custom_values() {
        let input = "https://example.test/v1\nmy-model\nMY_KEY\n20\n25000\n180\n";
        let mut reader = Cursor::new(input);
        let mut writer = Vec::new();
        let answers = prompt_config(&mut reader, &mut writer).expect("prompt");
        assert_eq!(
            answers,
            ConfigAnswers {
                base_url: "https://example.test/v1".to_string(),
                model: "my-model".to_string(),
                api_key_env: "MY_KEY".to_string(),
                max_iterations: 20,
                max_tokens: 25_000,
                max_duration_sec: 180,
            }
        );
    }

    #[test]
    fn render_agent_config_is_loadable() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("agent-config.example.yaml");
        let answers = ConfigAnswers {
            base_url: "https://example.test/v1".to_string(),
            model: "custom-model".to_string(),
            api_key_env: "CUSTOM_KEY".to_string(),
            max_iterations: 7,
            max_tokens: 9_000,
            max_duration_sec: 90,
        };
        fs::write(&path, render_agent_config(&answers)).expect("write");
        let (cfg, _) = load_config(&path).expect("load");
        assert_eq!(cfg.provider.base_url, "https://example.test/v1");
        assert_eq!(cfg.provider.model, "custom-model");
        assert_eq!(cfg.provider.api_key_env, "CUSTOM_KEY");
        assert_eq!(cfg.limits.max_iterations, 7);
        assert_eq!(cfg.limits.max_tokens, 9_000);
        assert_eq!(cfg.limits.max_duration_sec, 90);
    }
}
