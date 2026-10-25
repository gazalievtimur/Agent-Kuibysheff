//! `config document` get/set: the whole profile, with secrets redacted on the way out.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::{ConfigArgs, DocumentCmd};
use crate::commands::dotenv_file;

use super::common::{load_profile, save_profile_config};
use super::redact::{merge_config_put, redact_config};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct DocumentView {
    config: Value,
    api_key_env_set: bool,
    api_key_env: String,
}

#[derive(Debug, Serialize)]
struct DocumentSaved {
    ok: bool,
    config: Value,
}

#[derive(Debug, Deserialize)]
struct DocumentPut {
    config: Value,
    #[serde(default)]
    clear_api_key: bool,
}

pub fn run(args: &ConfigArgs, cmd: &DocumentCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        DocumentCmd::Get => {
            let profile = load_profile(&args.identity)?;
            let env_path = profile.paths.profile_dir.join(".env");
            let api_key_env = profile.config.provider.api_key_env.clone();
            let api_key_env_set = std::env::var_os(&api_key_env).is_some_and(|v| !v.is_empty())
                || dotenv_file::env_var_is_set_in_file(&env_path, &api_key_env)
                || profile.config.provider.has_inline_api_key();
            emit_ok(
                args.format,
                "document",
                "get",
                &DocumentView {
                    config: redact_config(&profile.config),
                    api_key_env_set,
                    api_key_env,
                },
            )
        }
        DocumentCmd::Set {
            from_file,
            clear_api_key,
        } => {
            let mut body = read_put(from_file.as_deref())?;
            if *clear_api_key {
                body.clear_api_key = true;
            }
            let mut profile = load_profile(&args.identity)?;
            let merged = merge_config_put(&profile.config, body.config, body.clear_api_key)
                .map_err(ConfigCmdError::message)?;
            save_profile_config(&profile.paths, &merged)?;
            profile.config = merged;
            emit_ok(
                args.format,
                "document",
                "set",
                &DocumentSaved {
                    ok: true,
                    config: redact_config(&profile.config),
                },
            )
        }
    }
}

fn read_put(from_file: Option<&Path>) -> Result<DocumentPut, ConfigCmdError> {
    let raw = match from_file {
        Some(path) => fs::read_to_string(path).map_err(ConfigCmdError::Io)?,
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    serde_json::from_str(&raw)
        .map_err(|err| ConfigCmdError::message(format!("invalid document JSON: {err}")))
}
