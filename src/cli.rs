use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "agent_Kuibyshev",
    version,
    about = "agent_Kuibyshev CLI worker"
)]
pub struct CliArgs {
    #[arg(long, value_name = "FILE")]
    pub config: PathBuf,

    #[arg(long, value_name = "DIR")]
    pub settings_dir: PathBuf,

    #[arg(long, value_name = "TEXT")]
    pub prompt: String,

    #[arg(long, value_name = "DIR")]
    pub home: PathBuf,

    #[arg(long, value_name = "PATH", num_args = 1.., action = clap::ArgAction::Append)]
    pub files: Vec<PathBuf>,

    #[arg(long)]
    pub max_iterations: Option<u32>,

    #[arg(long)]
    pub max_tokens: Option<u64>,

    #[arg(long)]
    pub max_duration_sec: Option<u64>,

    /// Persist the full unpruned chat transcript for this run.
    #[arg(long)]
    pub save_chat_history: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worker_inputs_and_multiple_files() {
        let args = CliArgs::try_parse_from([
            "agent",
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

        assert_eq!(args.prompt, "do work");
        assert_eq!(
            args.files,
            vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]
        );
    }

    #[test]
    fn save_chat_history_flag_is_available() {
        let args = CliArgs::try_parse_from([
            "agent",
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

        assert!(args.save_chat_history);
    }
}
