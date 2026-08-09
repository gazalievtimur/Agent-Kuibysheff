//! `config event-mcp` get/set.

use crate::cli::{ConfigArgs, EventMcpCmd};
use crate::event_mcp::EventMcpConfig;

use super::common::{load_profile, read_text_no_symlink, save_profile_config};
use super::{emit_ok, ConfigCmdError};

pub fn run(args: &ConfigArgs, cmd: &EventMcpCmd) -> Result<(), ConfigCmdError> {
    match cmd {
        EventMcpCmd::Get => {
            let profile = load_profile(&args.identity)?;
            emit_ok(args.format, "event_mcp", "get", &profile.config.event_mcp)
        }
        EventMcpCmd::Set { from_file } => {
            let raw = read_text_no_symlink(from_file)?;
            let event_mcp = parse_event_mcp_fragment(&raw, from_file)?;
            event_mcp
                .validate_shape()
                .map_err(ConfigCmdError::message)?;
            let mut profile = load_profile(&args.identity)?;
            profile.config.event_mcp = event_mcp;
            save_profile_config(&profile.paths, &profile.config)?;
            emit_ok(args.format, "event_mcp", "set", &profile.config.event_mcp)
        }
    }
}

fn parse_event_mcp_fragment(
    raw: &str,
    path: &std::path::Path,
) -> Result<EventMcpConfig, ConfigCmdError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => serde_json::from_str::<EventMcpConfig>(raw).map_err(|err| {
            ConfigCmdError::message(format!("failed to parse event-mcp JSON: {err}"))
        }),
        _ => serde_yaml::from_str::<EventMcpConfig>(raw)
            .or_else(|_| serde_json::from_str::<EventMcpConfig>(raw))
            .map_err(|err| {
                ConfigCmdError::message(format!("failed to parse event-mcp file: {err}"))
            }),
    }
}
