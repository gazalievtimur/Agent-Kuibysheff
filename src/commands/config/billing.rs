//! `config billing` get/set.

use crate::cli::{BillingCmd, ConfigArgs};
use crate::config::{BillingConfig, BillingMcpConfig};

use super::common::{load_profile, read_text_no_symlink, save_profile_config};
use super::{emit_ok, ConfigCmdError};

pub fn run(args: &ConfigArgs, cmd: &BillingCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        BillingCmd::Get => {
            let profile = load_profile(&args.identity)?;
            emit_ok(args.format, "billing", "get", &profile.config.billing)
        }
        BillingCmd::Set {
            from_file,
            provider_id,
            currency,
            mcp_target,
        } => {
            let mut profile = load_profile(&args.identity)?;
            if let Some(path) = from_file.as_ref() {
                let raw = read_text_no_symlink(path)?;
                profile.config.billing = parse_billing_fragment(&raw, path)?;
            } else {
                if provider_id.is_none() && currency.is_none() && mcp_target.is_none() {
                    return Err(ConfigCmdError::message(
                        "billing set requires `--from-file` or at least one field flag",
                    ));
                }
                if let Some(v) = provider_id {
                    profile.config.billing.provider_id = v.clone();
                }
                if let Some(v) = currency {
                    profile.config.billing.currency = v.clone();
                }
                if let Some(target) = mcp_target {
                    let timeout_ms = profile
                        .config
                        .billing
                        .mcp
                        .as_ref()
                        .map(|m| m.timeout_ms)
                        .unwrap_or_else(BillingMcpConfig::default_timeout_ms);
                    profile.config.billing.mcp = Some(BillingMcpConfig {
                        target: target.clone(),
                        timeout_ms,
                    });
                }
            }
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(args.format, "billing", "set", &profile.config.billing)
        }
    }
}

fn parse_billing_fragment(
    raw: &str,
    path: &std::path::Path,
) -> Result<BillingConfig, ConfigCmdError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => serde_json::from_str::<BillingConfig>(raw)
            .map_err(|err| ConfigCmdError::message(format!("failed to parse billing JSON: {err}"))),
        _ => serde_yaml::from_str::<BillingConfig>(raw)
            .or_else(|_| serde_json::from_str::<BillingConfig>(raw))
            .map_err(|err| ConfigCmdError::message(format!("failed to parse billing file: {err}"))),
    }
}
