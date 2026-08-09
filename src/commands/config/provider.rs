//! `config provider` get/set.

use serde::Serialize;

use crate::cli::{ConfigArgs, ProviderCmd};

use super::common::{load_profile, save_profile_config};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct ProviderView {
    base_url: String,
    model: String,
    api_key_env: String,
    has_inline_api_key: bool,
    timeout_ms: u64,
    max_retries: u32,
    retry_base_delay_ms: u64,
}

pub fn run(args: &ConfigArgs, cmd: &ProviderCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        ProviderCmd::Get => {
            let profile = load_profile(&args.identity)?;
            let view = provider_view(&profile.config.provider);
            emit_ok(args.format, "provider", "get", &view)
        }
        ProviderCmd::Set {
            base_url,
            model,
            api_key_env,
            timeout_ms,
            max_retries,
        } => {
            if base_url.is_none()
                && model.is_none()
                && api_key_env.is_none()
                && timeout_ms.is_none()
                && max_retries.is_none()
            {
                return Err(ConfigCmdError::message(
                    "provider set requires at least one flag",
                ));
            }
            let mut profile = load_profile(&args.identity)?;
            if let Some(v) = base_url {
                profile.config.provider.base_url = v.clone();
            }
            if let Some(v) = model {
                profile.config.provider.model = v.clone();
            }
            if let Some(v) = api_key_env {
                profile.config.provider.api_key_env = v.clone();
            }
            if let Some(v) = timeout_ms {
                profile.config.provider.timeout_ms = *v;
            }
            if let Some(v) = max_retries {
                profile.config.provider.max_retries = *v;
            }
            save_profile_config(&profile.paths, &profile.config)?;
            let view = provider_view(&profile.config.provider);
            emit_ok(args.format, "provider", "set", &view)
        }
    }
}

fn provider_view(p: &crate::config::ProviderConfig) -> ProviderView {
    ProviderView {
        base_url: p.base_url.clone(),
        model: p.model.clone(),
        api_key_env: p.api_key_env.clone(),
        has_inline_api_key: p.has_inline_api_key(),
        timeout_ms: p.timeout_ms,
        max_retries: p.max_retries,
        retry_base_delay_ms: p.retry_base_delay_ms,
    }
}
