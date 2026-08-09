//! Runtime agent configuration (YAML/JSON wire DTOs).

pub mod safety;

pub use safety::*;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::access::{
    resolve_access_policy, validate_access_config, AccessError, ResolvedAccessPolicy,
};
use crate::billing::Money;
use crate::cli::RunArgs;
use crate::limits::LimitsConfig;

// Access DTOs live in `access`; re-export so `config::AccessPolicyConfig` keeps working.
pub use crate::access::{
    AccessModeField, AccessPolicyConfig, FilesystemPolicyConfig, HomeFsPolicyConfig,
    ProgramPolicyConfig, RunPolicyConfig, ToolsPolicyConfig, WorkspacePolicyConfig,
};

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("failed to read config file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write config file `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(String),
    #[error("config validation failed: {0}")]
    Validation(String),
}

impl From<AccessError> for ConfigError {
    fn from(err: AccessError) -> Self {
        match err {
            AccessError::Validation(message) => Self::Validation(message),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    #[serde(default)]
    pub mcp: Vec<McpServerConfig>,
    #[serde(default)]
    pub event_mcp: crate::event_mcp::EventMcpConfig,
    #[serde(default)]
    pub billing: BillingConfig,
    pub limits: LimitsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Required capability policy. Use `access.mode: legacy` for permissive FS; otherwise
    /// enforcement is fail-closed (anything not listed is denied).
    #[serde(default)]
    pub access: Option<AccessPolicyConfig>,
}

/// Monetary accounting configuration. An omitted section still emits an
/// `unavailable` cost report rather than claiming a zero charge.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct BillingConfig {
    pub provider_id: String,
    pub currency: String,
    pub source_order: Vec<BillingSource>,
    pub provider_reported: ProviderReportedCostConfig,
    pub catalog_path: Option<PathBuf>,
    pub mcp: Option<BillingMcpConfig>,
    pub on_unpriced: BillingUnpricedPolicy,
}

impl Default for BillingConfig {
    fn default() -> Self {
        Self {
            provider_id: "openai_compatible".to_string(),
            currency: "USD".to_string(),
            source_order: vec![
                BillingSource::ProviderReported,
                BillingSource::Mcp,
                BillingSource::Catalog,
            ],
            provider_reported: ProviderReportedCostConfig::default(),
            catalog_path: None,
            mcp: None,
            on_unpriced: BillingUnpricedPolicy::Continue,
        }
    }
}

/// Ordered request-cost source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingSource {
    ProviderReported,
    Mcp,
    Catalog,
}

/// Provider response fields that may carry a charged amount.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct ProviderReportedCostConfig {
    /// Currency/unit assigned when the provider field itself has no unit.
    pub unit: Option<String>,
    /// JSON pointers checked in order against the completion response.
    pub json_pointers: Vec<String>,
    /// HTTP response headers checked in order.
    pub headers: Vec<String>,
}

impl Default for ProviderReportedCostConfig {
    fn default() -> Self {
        Self {
            unit: None,
            json_pointers: vec![
                "/usage/cost".to_string(),
                "/usage/response_cost/total_cost".to_string(),
            ],
            headers: vec!["x-litellm-response-cost".to_string()],
        }
    }
}

/// Optional MCP cost calculator binding.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BillingMcpConfig {
    /// Qualified discovered tool name (`server.tool`).
    pub target: String,
    #[serde(default = "BillingMcpConfig::default_timeout_ms")]
    pub timeout_ms: u64,
}

impl BillingMcpConfig {
    #[must_use]
    pub const fn default_timeout_ms() -> u64 {
        5_000
    }
}

/// Missing-price behavior. The initial implementation is deliberately fail-soft.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingUnpricedPolicy {
    #[default]
    Continue,
}

/// Working chat-window budgets for the configured model (not run stop limits).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ProviderHistoryConfig {
    /// Max messages kept after the fixed prefix (system + initial user).
    #[serde(default = "ProviderHistoryConfig::default_max_tail_messages")]
    pub max_tail_messages: usize,
    /// Max total UTF-8 characters across the pruned window (prefix + tail).
    #[serde(default = "ProviderHistoryConfig::default_max_chars")]
    pub max_chars: usize,
}

impl Default for ProviderHistoryConfig {
    fn default() -> Self {
        Self {
            max_tail_messages: Self::default_max_tail_messages(),
            max_chars: Self::default_max_chars(),
        }
    }
}

impl ProviderHistoryConfig {
    #[must_use]
    pub const fn default_max_tail_messages() -> usize {
        30
    }

    #[must_use]
    pub const fn default_max_chars() -> usize {
        200_000
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default = "ProviderConfig::default_api_key_env")]
    pub api_key_env: String,
    /// Inline API key for local configs. Prefer `.env` + `api_key_env` for shared setups.
    pub api_key: Option<String>,
    #[serde(default = "ProviderConfig::default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "ProviderConfig::default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "ProviderConfig::default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,
    /// Context-window pruning for this model. Independent of `limits.max_tokens`.
    #[serde(default)]
    pub history: ProviderHistoryConfig,
}

impl ProviderConfig {
    fn default_api_key_env() -> String {
        "OPENAI_API_KEY".to_string()
    }

    fn default_timeout_ms() -> u64 {
        60_000
    }
    fn default_max_retries() -> u32 {
        3
    }
    fn default_retry_base_delay_ms() -> u64 {
        500
    }

    #[must_use]
    pub fn has_inline_api_key(&self) -> bool {
        self.api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
    }

    /// Resolves the provider API key from inline config or an environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`std::env::VarError`] when neither `api_key` nor the configured
    /// environment variable is set.
    pub fn resolve_api_key(&self) -> Result<String, std::env::VarError> {
        if let Some(key) = &self.api_key {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
        std::env::var(&self.api_key_env)
    }
}

/// One configured MCP server (stdio subprocess or Streamable HTTP).
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "McpServerConfigRaw")]
pub struct McpServerConfig {
    pub name: String,
    pub timeout_ms: u64,
    pub transport: McpTransport,
}

impl McpServerConfig {
    #[must_use]
    pub const fn default_timeout_ms() -> u64 {
        20_000
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Serialize for McpServerConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.transport {
            McpTransport::Stdio(stdio) => {
                #[derive(Serialize)]
                struct Wire<'a> {
                    name: &'a str,
                    transport: &'a str,
                    command: &'a str,
                    #[serde(skip_serializing_if = "Vec::is_empty")]
                    args: &'a Vec<String>,
                    #[serde(skip_serializing_if = "HashMap::is_empty")]
                    env: &'a HashMap<String, String>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    cwd: &'a Option<PathBuf>,
                    timeout_ms: u64,
                }

                Wire {
                    name: &self.name,
                    transport: "stdio",
                    command: &stdio.command,
                    args: &stdio.args,
                    env: &stdio.env,
                    cwd: &stdio.cwd,
                    timeout_ms: self.timeout_ms,
                }
                .serialize(serializer)
            }
            McpTransport::Http(http) => {
                #[derive(Serialize)]
                struct Wire<'a> {
                    name: &'a str,
                    transport: &'a str,
                    url: &'a str,
                    #[serde(skip_serializing_if = "HashMap::is_empty")]
                    headers: &'a HashMap<String, String>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    auth: &'a Option<McpOAuthConfig>,
                    timeout_ms: u64,
                }

                Wire {
                    name: &self.name,
                    transport: "http",
                    url: &http.url,
                    headers: &http.headers,
                    auth: &http.auth,
                    timeout_ms: self.timeout_ms,
                }
                .serialize(serializer)
            }
        }
    }
}

/// Transport-specific MCP connection settings.
#[derive(Debug, Clone)]
pub enum McpTransport {
    Stdio(McpStdioConfig),
    Http(McpHttpConfig),
}

/// Local MCP server launched as a subprocess (stdio transport).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpStdioConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory for the child process. Relative `command` / `args` are
    /// resolved against this directory (then against the config file directory
    /// when unset). Absolute paths are left unchanged.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

/// Remote MCP server over Streamable HTTP (protocol 2025-11-25).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpHttpConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub auth: Option<McpOAuthConfig>,
}

/// OAuth 2.1 client settings for a protected Streamable HTTP MCP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthConfig {
    /// Static OAuth client_id. Mutually exclusive with `client_id_metadata_url`.
    pub client_id: Option<String>,
    /// Environment variable holding the client secret (confidential clients).
    pub client_secret_env: Option<String>,
    /// Client ID Metadata Document URL (SHOULD when AS supports CIMD).
    pub client_id_metadata_url: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Localhost callback port; `0` binds an ephemeral port.
    #[serde(default)]
    pub redirect_port: u16,
    /// Persistent token cache path (tilde/`~` expanded at runtime).
    pub token_store: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerConfigRaw {
    name: String,
    transport: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    url: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    auth: Option<McpOAuthConfig>,
    #[serde(default = "McpServerConfig::default_timeout_ms")]
    timeout_ms: u64,
}

impl TryFrom<McpServerConfigRaw> for McpServerConfig {
    type Error = String;

    fn try_from(raw: McpServerConfigRaw) -> Result<Self, Self::Error> {
        let has_command = raw.command.as_ref().is_some_and(|c| !c.trim().is_empty());
        let has_url = raw.url.as_ref().is_some_and(|u| !u.trim().is_empty());
        let kind = raw
            .transport
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);

        let transport = match kind.as_deref() {
            Some("stdio") => {
                if !has_command {
                    return Err("mcp transport `stdio` requires `command`".to_string());
                }
                if has_url {
                    return Err("mcp transport `stdio` must not set `url`".to_string());
                }
                if raw.auth.is_some() || !raw.headers.is_empty() {
                    return Err(
                        "mcp transport `stdio` must not set `url`/`headers`/`auth`".to_string()
                    );
                }
                McpTransport::Stdio(McpStdioConfig {
                    command: raw
                        .command
                        .ok_or_else(|| "mcp transport `stdio` requires `command`".to_string())?,
                    args: raw.args,
                    env: raw.env,
                    cwd: raw.cwd,
                })
            }
            Some("http") => {
                if !has_url {
                    return Err("mcp transport `http` requires `url`".to_string());
                }
                if has_command || !raw.args.is_empty() || !raw.env.is_empty() || raw.cwd.is_some() {
                    return Err(
                        "mcp transport `http` must not set `command`/`args`/`env`/`cwd`"
                            .to_string(),
                    );
                }
                McpTransport::Http(McpHttpConfig {
                    url: raw
                        .url
                        .ok_or_else(|| "mcp transport `http` requires `url`".to_string())?,
                    headers: raw.headers,
                    auth: raw.auth,
                })
            }
            Some(other) => {
                return Err(format!(
                    "unknown mcp transport `{other}` (expected `stdio` or `http`)"
                ));
            }
            None if has_url && !has_command => {
                if raw.cwd.is_some() || !raw.args.is_empty() || !raw.env.is_empty() {
                    return Err(
                        "mcp http entry must not set `command`/`args`/`env`/`cwd`".to_string()
                    );
                }
                McpTransport::Http(McpHttpConfig {
                    url: raw
                        .url
                        .ok_or_else(|| "mcp http entry requires `url`".to_string())?,
                    headers: raw.headers,
                    auth: raw.auth,
                })
            }
            None if has_command && !has_url => {
                if raw.auth.is_some() || !raw.headers.is_empty() {
                    return Err(
                        "stdio mcp servers must not set `headers`/`auth` (use `url` for HTTP)"
                            .to_string(),
                    );
                }
                McpTransport::Stdio(McpStdioConfig {
                    command: raw
                        .command
                        .ok_or_else(|| "mcp stdio entry requires `command`".to_string())?,
                    args: raw.args,
                    env: raw.env,
                    cwd: raw.cwd,
                })
            }
            None if has_url && has_command => {
                return Err("mcp entry must set either `command` or `url`, not both".to_string());
            }
            None => {
                return Err("mcp entry requires `command` (stdio) or `url` (http)".to_string());
            }
        };

        if let McpTransport::Http(http) = &transport {
            if let Some(auth) = &http.auth {
                let has_id = auth
                    .client_id
                    .as_ref()
                    .is_some_and(|id| !id.trim().is_empty());
                let has_meta = auth
                    .client_id_metadata_url
                    .as_ref()
                    .is_some_and(|u| !u.trim().is_empty());
                if has_id && has_meta {
                    return Err(
                        "mcp auth must set only one of `client_id` or `client_id_metadata_url`"
                            .to_string(),
                    );
                }
            }
        }

        Ok(Self {
            name: raw.name,
            timeout_ms: raw.timeout_ms,
            transport,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub enable_ai_log: bool,
    #[serde(default)]
    pub enable_mcp_log: bool,
    #[serde(default)]
    pub enable_chat_history: bool,
    /// Legacy directory override. Takes precedence over `sink.path` when set.
    pub output_dir: Option<PathBuf>,
    #[serde(default)]
    pub sink: LogSinkConfig,
    /// Redaction/truncation applied to AI/MCP JSONL audit payloads (not chat history).
    #[serde(default)]
    pub audit_redaction: AuditRedactionConfig,
}

fn default_audit_redaction_enabled() -> bool {
    true
}

fn default_audit_max_string_chars() -> usize {
    4096
}

/// Policy for scrubbing structured AI/MCP audit JSONL before it is written.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuditRedactionConfig {
    /// When `false`, payloads are written unmodified (legacy full audit).
    #[serde(default = "default_audit_redaction_enabled")]
    pub enabled: bool,
    /// UTF-8 character limit for string leaves; longer values get a truncation suffix.
    #[serde(default = "default_audit_max_string_chars")]
    pub max_string_chars: usize,
    /// Extra JSON object keys (case-insensitive) whose values become `[REDACTED]`.
    #[serde(default)]
    pub extra_sensitive_keys: Vec<String>,
}

impl Default for AuditRedactionConfig {
    fn default() -> Self {
        Self {
            enabled: default_audit_redaction_enabled(),
            max_string_chars: default_audit_max_string_chars(),
            extra_sensitive_keys: Vec::new(),
        }
    }
}

/// Destination for structured AI/MCP event logs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogSinkConfig {
    File { path: Option<PathBuf> },
    Db { connection_string: String },
}

impl Default for LogSinkConfig {
    fn default() -> Self {
        Self::File { path: None }
    }
}

/// Loads `.env` with deterministic precedence: process env (already set) wins,
/// then config-directory `.env`, then launch CWD `.env`.
///
/// Existing process environment variables are never overwritten. Missing files
/// are ignored; other I/O or parse failures are logged.
///
/// Callers should pass a config path already aligned with run resolution (including
/// `--project-root` → `{project}/.kuibysheff/...` when applicable).
pub fn load_dotenv() {
    load_dotenv_layered(None);
}

/// Like [`load_dotenv`], but prefers `{config_path.parent()}/.env` over CWD.
pub fn load_dotenv_layered(config_path: Option<&Path>) {
    if let Some(path) = config_path {
        if let Some(dir) = path.parent() {
            let env_file = dir.join(".env");
            match dotenvy::from_path(&env_file) {
                Ok(_) => {}
                Err(err) if err.not_found() => {}
                Err(err) => {
                    eprintln!(
                        "warning: failed to load .env from config dir `{}`: {err}",
                        env_file.display()
                    );
                }
            }
        }
    }
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(err) if err.not_found() => {}
        Err(err) => {
            eprintln!("warning: failed to load .env file: {err}");
        }
    }
}

/// Loads and validates runtime configuration from a YAML or JSON file.
///
/// Host paths declared under `access` are resolved relative to the config file directory
/// and compiled into an immutable [`ResolvedAccessPolicy`].
///
/// # Errors
///
/// Returns [`ConfigError`] if the file cannot be read, parsed, or fails validation.
pub fn load_config(path: &Path) -> Result<(AppConfig, ResolvedAccessPolicy), ConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;

    let cfg = parse_config_payload(&raw, path)?;
    validate(&cfg)?;
    let access = resolve_access_policy(cfg.access.as_ref(), config_parent_dir(path))?;
    Ok((cfg, access))
}

/// Deserializes config bytes without validation or access resolution.
///
/// Used by `config import` to treat an external path as an untrusted payload until
/// contents are written into the protected profile and validated there.
///
/// # Errors
///
/// Returns [`ConfigError::Parse`] when the payload is not valid YAML/JSON for [`AppConfig`].
pub fn parse_config_payload(raw: &str, path_hint: &Path) -> Result<AppConfig, ConfigError> {
    let extension = path_hint
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "json" => serde_json::from_str::<AppConfig>(raw)
            .map_err(|err| ConfigError::Parse(err.to_string())),
        "yaml" | "yml" => serde_yaml::from_str::<AppConfig>(raw)
            .map_err(|err| ConfigError::Parse(err.to_string())),
        _ => serde_yaml::from_str::<AppConfig>(raw)
            .or_else(|_| serde_json::from_str::<AppConfig>(raw))
            .map_err(|err| ConfigError::Parse(err.to_string())),
    }
}

/// Starter profile config with [`AccessPolicyConfig::minimal_profile`] grants.
#[must_use]
pub fn bootstrap_app_config() -> AppConfig {
    AppConfig {
        provider: ProviderConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            api_key: None,
            timeout_ms: 60_000,
            max_retries: 3,
            retry_base_delay_ms: 500,
            history: ProviderHistoryConfig::default(),
        },
        mcp: Vec::new(),
        event_mcp: crate::event_mcp::EventMcpConfig::default(),
        billing: BillingConfig::default(),
        limits: LimitsConfig {
            max_iterations: 10,
            max_tokens: 15_000,
            max_duration_sec: 120,
            max_cost: None,
        },
        logging: LoggingConfig {
            enable_ai_log: true,
            enable_mcp_log: true,
            enable_chat_history: false,
            output_dir: None,
            sink: LogSinkConfig::default(),
            ..Default::default()
        },
        access: Some(AccessPolicyConfig::minimal_profile()),
    }
}

/// Ensures `cfg.access` is present, filling [`AccessPolicyConfig::minimal_profile`] when omitted.
///
/// Returns `true` when the payload already declared `access`.
pub fn ensure_access_present(cfg: &mut AppConfig) -> bool {
    if cfg.access.is_some() {
        true
    } else {
        cfg.access = Some(AccessPolicyConfig::minimal_profile());
        false
    }
}

/// Serializes and atomically writes `cfg` to `path` as YAML or JSON by extension.
///
/// YAML files (`.yaml` / `.yml`, or unrecognized extensions) get an optional header
/// comment. JSON is used for `.json`. Validation and [`ConfigSafetyValidator`] run
/// before any bytes are written.
///
/// # Errors
///
/// Returns [`ConfigError`] when validation/safety fails or the file cannot be written.
pub fn save_config(path: &Path, cfg: &AppConfig) -> Result<(), ConfigError> {
    validate(cfg)?;
    ConfigSafetyValidator::check(cfg)?;

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let contents = match extension.as_str() {
        "json" => {
            serde_json::to_string_pretty(cfg).map_err(|err| ConfigError::Parse(err.to_string()))?
        }
        _ => {
            // `.yaml` / `.yml` and unrecognized extensions serialize as YAML.
            let body =
                serde_yaml::to_string(cfg).map_err(|err| ConfigError::Parse(err.to_string()))?;
            format!("# Managed by agent_Kuibysheff config\n{body}")
        }
    };

    atomic_write(path, contents.as_bytes())
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    let parent = config_parent_dir(path);
    if !parent.as_os_str().is_empty() && parent != Path::new(".") {
        fs::create_dir_all(parent).map_err(|source| ConfigError::WriteFile {
            path: path.display().to_string(),
            source,
        })?;
    }

    let file_stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let tmp_name = format!(
        ".{file_stem}.{}.{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp_path = parent.join(tmp_name);

    if let Err(source) = fs::write(&tmp_path, contents) {
        let _ = fs::remove_file(&tmp_path);
        return Err(ConfigError::WriteFile {
            path: path.display().to_string(),
            source,
        });
    }

    // Windows `rename` does not replace an existing destination.
    if path.exists() {
        if let Err(source) = fs::remove_file(path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(ConfigError::WriteFile {
                path: path.display().to_string(),
                source,
            });
        }
    }

    if let Err(source) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(ConfigError::WriteFile {
            path: path.display().to_string(),
            source,
        });
    }

    Ok(())
}

/// Returns the directory that relative `access` host paths resolve against.
#[must_use]
pub fn config_parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub fn apply_cli_overrides(cfg: &mut AppConfig, cli: &RunArgs) {
    apply_limit_overrides(
        cfg,
        cli.max_iterations,
        cli.max_tokens,
        cli.max_duration_sec,
        cli.max_cost.clone(),
        cli.save_chat_history,
    );
}

/// Apply optional limit / chat-history overrides shared by `run` and `acp`.
pub fn apply_limit_overrides(
    cfg: &mut AppConfig,
    max_iterations: Option<u32>,
    max_tokens: Option<u64>,
    max_duration_sec: Option<u64>,
    max_cost: Option<Money>,
    save_chat_history: bool,
) {
    if let Some(max_iterations) = max_iterations {
        cfg.limits.max_iterations = max_iterations;
    }
    if let Some(max_tokens) = max_tokens {
        cfg.limits.max_tokens = max_tokens;
    }
    if let Some(max_duration_sec) = max_duration_sec {
        cfg.limits.max_duration_sec = max_duration_sec;
    }
    if let Some(max_cost) = max_cost {
        cfg.limits.max_cost = Some(max_cost);
    }
    if save_chat_history {
        cfg.logging.enable_chat_history = true;
    }
}

/// Validates required fields and MCP server configuration.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] when a required field is missing or invalid.
pub fn validate(cfg: &AppConfig) -> Result<(), ConfigError> {
    if cfg.provider.base_url.trim().is_empty() {
        return Err(ConfigError::Validation(
            "`provider.base_url` must not be empty".to_string(),
        ));
    }
    if cfg.provider.model.trim().is_empty() {
        return Err(ConfigError::Validation(
            "`provider.model` must not be empty".to_string(),
        ));
    }
    if cfg.provider.api_key_env.trim().is_empty() && !cfg.provider.has_inline_api_key() {
        return Err(ConfigError::Validation(
            "set either `provider.api_key` or non-empty `provider.api_key_env`".to_string(),
        ));
    }
    if cfg.provider.timeout_ms == 0 {
        return Err(ConfigError::Validation(
            "`provider.timeout_ms` must be > 0".to_string(),
        ));
    }
    if cfg.provider.history.max_tail_messages == 0 {
        return Err(ConfigError::Validation(
            "`provider.history.max_tail_messages` must be > 0".to_string(),
        ));
    }
    if cfg.provider.history.max_chars == 0 {
        return Err(ConfigError::Validation(
            "`provider.history.max_chars` must be > 0".to_string(),
        ));
    }
    if cfg.limits.max_iterations == 0 {
        return Err(ConfigError::Validation(
            "`limits.max_iterations` must be > 0".to_string(),
        ));
    }
    if cfg.limits.max_tokens == 0 {
        return Err(ConfigError::Validation(
            "`limits.max_tokens` must be > 0".to_string(),
        ));
    }
    if cfg.limits.max_duration_sec == 0 {
        return Err(ConfigError::Validation(
            "`limits.max_duration_sec` must be > 0".to_string(),
        ));
    }
    Money::new(Decimal::ZERO, cfg.billing.currency.clone())
        .map_err(|error| ConfigError::Validation(format!("`billing.currency`: {error}")))?;
    if cfg.billing.provider_id.trim().is_empty() {
        return Err(ConfigError::Validation(
            "`billing.provider_id` must not be empty".to_string(),
        ));
    }
    let mut billing_sources = HashSet::new();
    for source in &cfg.billing.source_order {
        if !billing_sources.insert(*source) {
            return Err(ConfigError::Validation(
                "`billing.source_order` must not contain duplicates".to_string(),
            ));
        }
    }
    if cfg.billing.source_order.is_empty() {
        return Err(ConfigError::Validation(
            "`billing.source_order` must not be empty".to_string(),
        ));
    }
    if let Some(unit) = &cfg.billing.provider_reported.unit {
        Money::new(Decimal::ZERO, unit.clone()).map_err(|error| {
            ConfigError::Validation(format!("`billing.provider_reported.unit`: {error}"))
        })?;
    }
    for pointer in &cfg.billing.provider_reported.json_pointers {
        if !pointer.starts_with('/') {
            return Err(ConfigError::Validation(format!(
                "billing JSON pointer `{pointer}` must start with `/`"
            )));
        }
    }
    if cfg
        .billing
        .provider_reported
        .headers
        .iter()
        .any(|header| header.trim().is_empty())
    {
        return Err(ConfigError::Validation(
            "billing provider-reported headers must not be empty".to_string(),
        ));
    }
    if let Some(path) = &cfg.billing.catalog_path {
        if path.as_os_str().is_empty() {
            return Err(ConfigError::Validation(
                "`billing.catalog_path` must not be empty".to_string(),
            ));
        }
    }
    if let Some(limit) = &cfg.limits.max_cost {
        if limit.amount() <= Decimal::ZERO {
            return Err(ConfigError::Validation(
                "`limits.max_cost.amount` must be > 0".to_string(),
            ));
        }
        if limit.currency() != cfg.billing.currency {
            return Err(ConfigError::Validation(format!(
                "`limits.max_cost.currency` must equal `billing.currency` ({})",
                cfg.billing.currency
            )));
        }
    }
    let mut names = HashSet::new();
    for server in &cfg.mcp {
        if server.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "each `mcp[].name` must not be empty".to_string(),
            ));
        }
        if !names.insert(server.name.clone()) {
            return Err(ConfigError::Validation(format!(
                "duplicate mcp server name `{}`",
                server.name
            )));
        }
        if server.timeout_ms == 0 {
            return Err(ConfigError::Validation(format!(
                "`mcp[{name}].timeout_ms` must be > 0",
                name = server.name
            )));
        }
        match &server.transport {
            McpTransport::Stdio(stdio) => {
                if stdio.command.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "`mcp[{name}].command` must not be empty",
                        name = server.name
                    )));
                }
            }
            McpTransport::Http(http) => {
                let parsed = url::Url::parse(http.url.trim()).map_err(|err| {
                    ConfigError::Validation(format!(
                        "`mcp[{name}].url` is invalid: {err}",
                        name = server.name
                    ))
                })?;
                if parsed.scheme() != "http" && parsed.scheme() != "https" {
                    return Err(ConfigError::Validation(format!(
                        "`mcp[{name}].url` must use http or https",
                        name = server.name
                    )));
                }
                if let Some(auth) = &http.auth {
                    if let Some(meta) = &auth.client_id_metadata_url {
                        url::Url::parse(meta.trim()).map_err(|err| {
                            ConfigError::Validation(format!(
                                "`mcp[{name}].auth.client_id_metadata_url` is invalid: {err}",
                                name = server.name
                            ))
                        })?;
                    }
                }
            }
        }
    }

    if let Some(mcp) = &cfg.billing.mcp {
        if mcp.timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "`billing.mcp.timeout_ms` must be > 0".to_string(),
            ));
        }
        let Some((server, tool)) = mcp.target.split_once('.') else {
            return Err(ConfigError::Validation(
                "`billing.mcp.target` must be a qualified `server.tool` name".to_string(),
            ));
        };
        if server.trim().is_empty() || tool.trim().is_empty() || tool.contains('.') {
            return Err(ConfigError::Validation(
                "`billing.mcp.target` must be a qualified `server.tool` name".to_string(),
            ));
        }
        if !names.contains(server) {
            return Err(ConfigError::Validation(format!(
                "`billing.mcp.target` references unknown MCP server `{server}`"
            )));
        }
        if cfg
            .event_mcp
            .events
            .values()
            .flat_map(|pipeline| &pipeline.handlers)
            .any(|handler| {
                handler
                    .target
                    .split_once('.')
                    .is_some_and(|(event_server, _)| event_server == server)
            })
        {
            return Err(ConfigError::Validation(format!(
                "billing MCP server `{server}` must be dedicated and cannot host Event-MCP handlers"
            )));
        }
    }

    cfg.event_mcp
        .validate_shape()
        .map_err(ConfigError::Validation)?;

    validate_access_config(
        cfg.access.as_ref(),
        cfg.mcp.iter().map(|server| server.name.as_str()),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AppConfig {
        AppConfig {
            provider: ProviderConfig {
                base_url: "https://example.com/v1".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                api_key: None,
                timeout_ms: 1000,
                max_retries: 2,
                retry_base_delay_ms: 100,
                history: ProviderHistoryConfig::default(),
            },
            mcp: vec![McpServerConfig {
                name: "local".to_string(),
                timeout_ms: 1000,
                transport: McpTransport::Stdio(McpStdioConfig {
                    command: "mcp-server".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                    cwd: None,
                }),
            }],
            event_mcp: crate::event_mcp::EventMcpConfig::default(),
            billing: BillingConfig::default(),
            limits: LimitsConfig {
                max_iterations: 5,
                max_tokens: 500,
                max_duration_sec: 30,
                max_cost: None,
            },
            logging: LoggingConfig {
                enable_ai_log: false,
                enable_mcp_log: false,
                enable_chat_history: false,
                output_dir: None,
                sink: LogSinkConfig::default(),
                ..Default::default()
            },
            access: Some(AccessPolicyConfig::default()),
        }
    }

    #[test]
    fn config_validation_rejects_empty_model() {
        let mut cfg = sample_config();
        cfg.provider.model.clear();
        let err = validate(&cfg).expect_err("empty model");
        assert!(
            err.to_string().contains("provider.model"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn config_validation_rejects_max_cost_currency_mismatch() {
        let mut cfg = sample_config();
        cfg.limits.max_cost = Some(Money::parse("0.00000894", "EUR").expect("money"));
        let error = validate(&cfg).expect_err("currency mismatch");
        assert!(error.to_string().contains("max_cost.currency"), "{error}");
    }

    #[test]
    fn config_validation_accepts_dedicated_billing_mcp_target() {
        let mut cfg = sample_config();
        cfg.billing.mcp = Some(BillingMcpConfig {
            target: "local.calculate".to_string(),
            timeout_ms: 100,
        });
        validate(&cfg).expect("dedicated billing target");
    }

    #[test]
    fn config_validation_rejects_zero_history_budgets() {
        let mut cfg = sample_config();
        cfg.provider.history.max_tail_messages = 0;
        let err = validate(&cfg).expect_err("zero max_tail_messages");
        assert!(
            err.to_string()
                .contains("provider.history.max_tail_messages"),
            "unexpected error: {err}"
        );

        let mut cfg = sample_config();
        cfg.provider.history.max_chars = 0;
        let err = validate(&cfg).expect_err("zero max_chars");
        assert!(
            err.to_string().contains("provider.history.max_chars"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn provider_history_defaults_when_section_omitted() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";
        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert_eq!(cfg.provider.history, ProviderHistoryConfig::default());
    }

    #[test]
    fn logging_config_parses_legacy_output_dir() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  enable_ai_log: true
  output_dir: ./legacy-logs
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert_eq!(cfg.logging.output_dir, Some(PathBuf::from("./legacy-logs")));
        assert!(matches!(
            cfg.logging.sink,
            LogSinkConfig::File { path: None }
        ));
    }

    #[test]
    fn logging_config_parses_file_sink_path() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  sink:
    type: file
    path: ./custom-logs
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(matches!(
            cfg.logging.sink,
            LogSinkConfig::File {
                path: Some(ref path)
            } if path == &PathBuf::from("./custom-logs")
        ));
    }

    #[test]
    fn provider_config_accepts_inline_api_key() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key: inline-secret
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(cfg.provider.has_inline_api_key());
        assert_eq!(
            cfg.provider.resolve_api_key().expect("inline key"),
            "inline-secret"
        );
    }

    #[test]
    fn logging_config_parses_chat_history_flag() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  enable_chat_history: true
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(cfg.logging.enable_chat_history);
    }

    #[test]
    fn logging_config_audit_redaction_defaults_and_override() {
        let yaml_default = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  enable_ai_log: true
";
        let cfg = serde_yaml::from_str::<AppConfig>(yaml_default).expect("parse");
        assert!(cfg.logging.audit_redaction.enabled);
        assert_eq!(cfg.logging.audit_redaction.max_string_chars, 4096);
        assert!(cfg.logging.audit_redaction.extra_sensitive_keys.is_empty());

        let yaml_override = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
logging:
  audit_redaction:
    enabled: false
    max_string_chars: 128
    extra_sensitive_keys: [session_token]
";
        let cfg = serde_yaml::from_str::<AppConfig>(yaml_override).expect("parse");
        assert!(!cfg.logging.audit_redaction.enabled);
        assert_eq!(cfg.logging.audit_redaction.max_string_chars, 128);
        assert_eq!(
            cfg.logging.audit_redaction.extra_sensitive_keys,
            vec!["session_token".to_string()]
        );
    }

    #[test]
    fn config_rejects_legacy_agent_fields() {
        let yaml = r"
goal: legacy
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";

        let err = serde_yaml::from_str::<AppConfig>(yaml).expect_err("legacy field");
        assert!(err.to_string().contains("goal"), "unexpected error: {err}");
    }

    #[test]
    fn config_without_access_is_rejected() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        assert!(cfg.access.is_none());
        let err = validate(&cfg).expect_err("missing access");
        assert!(
            err.to_string().contains("`access` is required"),
            "unexpected error: {err}"
        );
        let err = resolve_access_policy(None, Path::new(".")).expect_err("missing access");
        assert!(err.to_string().contains("`access` is required"));
    }

    #[test]
    fn explicit_legacy_access_mode_resolves() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  mode: legacy
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        validate(&cfg).expect("validate");
        let policy = resolve_access_policy(cfg.access.as_ref(), Path::new(".")).expect("legacy");
        assert!(policy.is_legacy());
        assert!(!policy.allows_builtin(&crate::access::QualifiedTool::parse("home.run").unwrap()));
        assert!(policy.allows_builtin(&crate::access::QualifiedTool::parse("home.read").unwrap()));
    }

    #[test]
    fn legacy_mode_rejects_mixed_grants() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  mode: legacy
  tools:
    builtins: [home.read]
";

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        let err = validate(&cfg).expect_err("mixed legacy");
        assert!(
            err.to_string().contains("mode: legacy"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn strict_access_section_parses_and_validates() {
        let yaml = r#"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
  filesystem:
    home:
      read: ["in", "out"]
      write: ["out"]
  run:
    max_args: 16
    max_arg_chars: 1024
"#;

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        let access = cfg.access.as_ref().expect("access");
        assert_eq!(access.tools.builtins.len(), 3);
        assert_eq!(access.filesystem.home.read, ["in", "out"]);
        assert_eq!(access.run.max_args, 16);
        validate(&cfg).expect("validate");
    }

    #[test]
    fn access_rejects_unknown_fields() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  network:
    allow: true
";

        let err = serde_yaml::from_str::<AppConfig>(yaml).expect_err("unknown field");
        assert!(
            err.to_string().contains("network"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn access_rejects_malformed_home_grant() {
        let yaml = r#"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  filesystem:
    home:
      read: ["../escape"]
"#;

        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        let err = validate(&cfg).expect_err("parent grant");
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn access_rejects_duplicate_program_alias() {
        let mut cfg = sample_config();
        cfg.access = Some(AccessPolicyConfig {
            run: RunPolicyConfig {
                programs: vec![
                    ProgramPolicyConfig {
                        name: "python".to_string(),
                        executable: PathBuf::from("a.exe"),
                        runtime_read_roots: Vec::new(),
                        inherit_env: Vec::new(),
                        allow_children: false,
                    },
                    ProgramPolicyConfig {
                        name: "python".to_string(),
                        executable: PathBuf::from("b.exe"),
                        runtime_read_roots: Vec::new(),
                        inherit_env: Vec::new(),
                        allow_children: false,
                    },
                ],
                ..RunPolicyConfig::default()
            },
            ..AccessPolicyConfig::default()
        });
        let err = validate(&cfg).expect_err("duplicate");
        assert!(err.to_string().contains("duplicate program alias"));
    }

    #[test]
    fn access_rejects_reserved_mcp_server_names() {
        let mut cfg = sample_config();
        cfg.mcp[0].name = "home".to_string();
        let err = validate(&cfg).expect_err("reserved");
        assert!(err.to_string().contains("reserved"));
    }

    #[test]
    fn load_config_resolves_access_relative_to_config_dir() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let inputs = dir.path().join("inputs");
        fs::create_dir_all(&inputs).expect("inputs");

        let config_path = dir.path().join("agent-config.yaml");
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
access:
  tools:
    builtins: [home.read]
  filesystem:
    home:
      read: [in]
    input_roots: [inputs]
";
        {
            let mut file = fs::File::create(&config_path).expect("create config");
            write!(file, "{yaml}").expect("write");
        }

        let (cfg, policy) = load_config(&config_path).expect("load");
        assert!(cfg.access.is_some());
        assert!(!policy.is_legacy());
        assert_eq!(
            policy.input_roots()[0].as_path(),
            fs::canonicalize(&inputs).unwrap().as_path()
        );
    }

    #[test]
    fn example_config_file_loads() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("agent-config.example.yaml");
        let (cfg, policy) = load_config(&path).expect("load agent-config.example.yaml");
        assert!(cfg.access.is_some(), "example should demonstrate access");
        assert!(!policy.is_legacy());
        assert!(policy.allows_builtin(&crate::access::QualifiedTool::parse("home.run").unwrap()));
        assert!(policy.programs().is_empty());
    }

    #[test]
    fn mcp_parses_stdio_without_transport_tag() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
mcp:
  - name: local
    command: mcp-server
    args: [--flag]
access:
  mode: strict
";
        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        validate(&cfg).expect("valid");
        match &cfg.mcp[0].transport {
            McpTransport::Stdio(stdio) => {
                assert_eq!(stdio.command, "mcp-server");
                assert_eq!(stdio.args, vec!["--flag"]);
            }
            McpTransport::Http(_) => panic!("expected stdio"),
        }
    }

    #[test]
    fn mcp_parses_http_with_auth() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
mcp:
  - name: remote
    transport: http
    url: https://mcp.example.com/mcp
    headers:
      X-Test: one
    auth:
      client_id: agent-kuibysheff
      scopes: [mcp:tools]
      redirect_port: 0
      token_store: ./tokens.json
access:
  mode: strict
";
        let cfg = serde_yaml::from_str::<AppConfig>(yaml).expect("parse");
        validate(&cfg).expect("valid");
        match &cfg.mcp[0].transport {
            McpTransport::Http(http) => {
                assert_eq!(http.url, "https://mcp.example.com/mcp");
                assert_eq!(http.headers.get("X-Test").map(String::as_str), Some("one"));
                let auth = http.auth.as_ref().expect("auth");
                assert_eq!(auth.client_id.as_deref(), Some("agent-kuibysheff"));
            }
            McpTransport::Stdio(_) => panic!("expected http"),
        }
    }

    #[test]
    fn mcp_rejects_command_and_url_together() {
        let yaml = r"
provider:
  base_url: https://example.com/v1
  model: test
  api_key_env: TEST_KEY
limits:
  max_iterations: 1
  max_tokens: 1
  max_duration_sec: 1
mcp:
  - name: bad
    command: mcp-server
    url: https://mcp.example.com/mcp
";
        let err = serde_yaml::from_str::<AppConfig>(yaml).expect_err("command and url");
        assert!(
            err.to_string().contains("command") || err.to_string().contains("url"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn save_config_round_trips_yaml() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("agent-config.yaml");
        let cfg = sample_config();
        save_config(&path, &cfg).expect("save");
        let raw = fs::read_to_string(&path).expect("read");
        assert!(raw.starts_with("# Managed by agent_Kuibysheff config\n"));
        let (loaded, _) = load_config(&path).expect("load");
        assert_eq!(loaded.provider.model, cfg.provider.model);
        assert_eq!(loaded.mcp[0].name, "local");
        match &loaded.mcp[0].transport {
            McpTransport::Stdio(stdio) => assert_eq!(stdio.command, "mcp-server"),
            McpTransport::Http(_) => panic!("expected stdio"),
        }
    }
}
