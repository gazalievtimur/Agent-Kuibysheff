//! `config secret set`: write one env var into the profile `.env` (value from stdin).

use std::io::{self, Read};

use serde_json::json;

use crate::cli::{ConfigArgs, SecretCmd};

use super::common::load_profile;
use super::{emit_ok, ConfigCmdError};
use crate::commands::dotenv_file::upsert_env_var;

pub fn run(args: &ConfigArgs, cmd: &SecretCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        SecretCmd::Set { key } => {
            let profile = load_profile(&args.identity)?;
            let value = read_stdin_secret()?;
            if value.is_empty() {
                return Err(ConfigCmdError::message("secret value must not be empty"));
            }
            let env_path = profile.paths.profile_dir.join(".env");
            upsert_env_var(&env_path, key, &value)
                .map_err(|err| ConfigCmdError::message(err.to_string()))?;
            emit_ok(
                args.format,
                "secret",
                "set",
                &json!({ "ok": true, "key": key }),
            )
        }
    }
}

/// Read a secret from stdin, dropping a single trailing newline from a pipe.
fn read_stdin_secret() -> Result<String, ConfigCmdError> {
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    if raw.ends_with("\r\n") {
        raw.truncate(raw.len() - 2);
    } else if raw.ends_with('\n') {
        raw.pop();
    }
    Ok(raw)
}
