//! `config prompt` get/set.

use serde::Serialize;

use crate::cli::{ConfigArgs, PromptCmd};
use crate::settings::write_master_prompt;

use super::common::{load_profile, read_cli_text};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct PromptView {
    master_prompt: String,
}

pub fn run(args: &ConfigArgs, cmd: &PromptCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        PromptCmd::Get => {
            let profile = load_profile(&args.identity)?;
            emit_ok(
                args.format,
                "prompt",
                "get",
                &PromptView {
                    master_prompt: profile.settings.master_prompt,
                },
            )
        }
        PromptCmd::Set { text, file, stdin } => {
            let content = read_cli_text(text.as_deref(), file.as_deref(), *stdin, "prompt")?;
            let profile = load_profile(&args.identity)?;
            write_master_prompt(&profile.paths.settings_dir, &content)?;
            emit_ok(
                args.format,
                "prompt",
                "set",
                &PromptView {
                    master_prompt: content,
                },
            )
        }
    }
}
