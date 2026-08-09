//! `config prompt` get/set.

use serde::Serialize;

use crate::cli::{ConfigArgs, PromptCmd};
use crate::settings::write_master_prompt;

use super::common::{load_profile, read_text_no_symlink};
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
        PromptCmd::Set { text, file } => {
            let content = match (text.as_ref(), file.as_ref()) {
                (Some(t), None) => (*t).clone(),
                (None, Some(path)) => read_text_no_symlink(path)?,
                (Some(_), Some(_)) => {
                    return Err(ConfigCmdError::message(
                        "prompt set: pass either `--text` or `--file`, not both",
                    ));
                }
                (None, None) => {
                    return Err(ConfigCmdError::message(
                        "prompt set requires `--text` or `--file`",
                    ));
                }
            };
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
