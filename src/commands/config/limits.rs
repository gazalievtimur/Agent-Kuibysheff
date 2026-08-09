//! `config limits` get/set.

use std::str::FromStr;

use serde::Serialize;

use crate::billing::Money;
use crate::cli::{ConfigArgs, LimitsCmd};

use super::common::{load_profile, save_profile_config};
use super::{emit_ok, ConfigCmdError};

#[derive(Debug, Serialize)]
struct LimitsView {
    max_iterations: u32,
    max_tokens: u64,
    max_duration_sec: u64,
    max_cost: Option<String>,
}

pub fn run(args: &ConfigArgs, cmd: &LimitsCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        LimitsCmd::Get => {
            let profile = load_profile(&args.identity)?;
            emit_ok(
                args.format,
                "limits",
                "get",
                &limits_view(&profile.config.limits),
            )
        }
        LimitsCmd::Set {
            max_iterations,
            max_tokens,
            max_duration_sec,
            max_cost,
        } => {
            if max_iterations.is_none()
                && max_tokens.is_none()
                && max_duration_sec.is_none()
                && max_cost.is_none()
            {
                return Err(ConfigCmdError::message(
                    "limits set requires at least one flag",
                ));
            }
            let mut profile = load_profile(&args.identity)?;
            if let Some(v) = max_iterations {
                profile.config.limits.max_iterations = *v;
            }
            if let Some(v) = max_tokens {
                profile.config.limits.max_tokens = *v;
            }
            if let Some(v) = max_duration_sec {
                profile.config.limits.max_duration_sec = *v;
            }
            if let Some(raw) = max_cost.as_ref() {
                let money = Money::from_str(raw)
                    .map_err(|err| ConfigCmdError::message(format!("invalid --max-cost: {err}")))?;
                profile.config.limits.max_cost = Some(money);
            }
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(
                args.format,
                "limits",
                "set",
                &limits_view(&profile.config.limits),
            )
        }
    }
}

fn limits_view(limits: &crate::limits::LimitsConfig) -> LimitsView {
    LimitsView {
        max_iterations: limits.max_iterations,
        max_tokens: limits.max_tokens,
        max_duration_sec: limits.max_duration_sec,
        max_cost: limits
            .max_cost
            .as_ref()
            .map(|m| format!("{}:{}", m.currency(), m.amount())),
    }
}
