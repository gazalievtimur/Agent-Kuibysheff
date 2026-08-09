//! `config rules` get/set/clear.

use serde::Serialize;

use crate::cli::{ConfigArgs, RulesCmd};
use crate::settings::{clear_rules, write_rules};

use super::common::{load_profile, read_text_no_symlink};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct RulesView {
    rules: String,
}

pub fn run(args: &ConfigArgs, cmd: &RulesCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        RulesCmd::Get => {
            let profile = load_profile(&args.identity)?;
            emit_ok(
                args.format,
                "rules",
                "get",
                &RulesView {
                    rules: profile.settings.rules,
                },
            )
        }
        RulesCmd::Set { text, file } => {
            let content = match (text.as_ref(), file.as_ref()) {
                (Some(t), None) => (*t).clone(),
                (None, Some(path)) => read_text_no_symlink(path)?,
                (Some(_), Some(_)) => {
                    return Err(ConfigCmdError::message(
                        "rules set: pass either `--text` or `--file`, not both",
                    ));
                }
                (None, None) => {
                    return Err(ConfigCmdError::message(
                        "rules set requires `--text` or `--file`",
                    ));
                }
            };
            let profile = load_profile(&args.identity)?;
            write_rules(&profile.paths.settings_dir, &content)?;
            emit_ok(args.format, "rules", "set", &RulesView { rules: content })
        }
        RulesCmd::Clear => {
            let profile = load_profile(&args.identity)?;
            clear_rules(&profile.paths.settings_dir)?;
            emit_ok(
                args.format,
                "rules",
                "clear",
                &RulesView {
                    rules: String::new(),
                },
            )
        }
    }
}
