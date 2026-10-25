//! `config` management CLI (CRUD over the protected agent profile).

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use super::AgentIdentityArgs;

/// Output format for management commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ConfigFormat {
    #[default]
    Text,
    Json,
}

/// Top-level `config` arguments.
#[derive(Debug, Parser)]
pub struct ConfigArgs {
    #[command(flatten)]
    pub identity: AgentIdentityArgs,

    #[arg(long, value_enum, default_value_t = ConfigFormat::Text)]
    pub format: ConfigFormat,

    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Copy external config/settings into the protected agent profile.
    Import(ImportArgs),
    /// Show a short profile summary.
    Show,
    /// Read or replace the whole agent config document (secrets stay redacted).
    #[command(subcommand)]
    Document(DocumentCmd),
    /// Write a profile `.env` secret. The value is read from stdin, not argv.
    #[command(subcommand)]
    Secret(SecretCmd),
    #[command(subcommand)]
    Provider(ProviderCmd),
    #[command(subcommand)]
    Limits(LimitsCmd),
    #[command(subcommand)]
    Access(AccessCmd),
    #[command(subcommand)]
    Mcp(McpCmd),
    #[command(subcommand)]
    Billing(BillingCmd),
    /// Event-MCP middleware configuration.
    #[command(name = "event-mcp", subcommand)]
    EventMcp(EventMcpCmd),
    #[command(subcommand)]
    Skill(SkillCmd),
    #[command(subcommand)]
    Prompt(PromptCmd),
    #[command(subcommand)]
    Rules(RulesCmd),
    #[command(subcommand)]
    Tools(ToolsCmd),
}

#[derive(Debug, Parser)]
pub struct ImportArgs {
    /// External config file or settings/agent directory bundle.
    #[arg(long, value_name = "PATH")]
    pub from: PathBuf,
    /// Overwrite an existing non-empty profile.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum ProviderCmd {
    Get,
    Set {
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        api_key_env: Option<String>,
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long)]
        max_retries: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
pub enum LimitsCmd {
    Get,
    Set {
        #[arg(long)]
        max_iterations: Option<u32>,
        #[arg(long)]
        max_tokens: Option<u64>,
        #[arg(long)]
        max_duration_sec: Option<u64>,
        #[arg(long, value_name = "CURRENCY:AMOUNT")]
        max_cost: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccessCmd {
    Get,
    /// Replace the entire `access` section from a YAML/JSON file.
    Set {
        #[arg(long, value_name = "FILE")]
        from_file: PathBuf,
    },
    #[command(subcommand)]
    Builtins(BuiltinsCmd),
}

#[derive(Debug, Subcommand)]
pub enum BuiltinsCmd {
    List,
    Add { tool: String },
    Remove { tool: String },
}

#[derive(Debug, Subcommand)]
pub enum McpCmd {
    List,
    Get {
        name: String,
    },
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        command: Option<String>,
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long = "env", value_name = "K=V")]
        env: Vec<String>,
        #[arg(long = "header", value_name = "K=V")]
        headers: Vec<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        timeout_ms: Option<u64>,
    },
    Set {
        name: String,
        #[arg(long)]
        command: Option<String>,
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        timeout_ms: Option<u64>,
    },
    Remove {
        name: String,
    },
    /// Connect and list discovered tools.
    Tools {
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum BillingCmd {
    Get,
    Set {
        #[arg(long, value_name = "FILE")]
        from_file: Option<PathBuf>,
        #[arg(long)]
        provider_id: Option<String>,
        #[arg(long)]
        currency: Option<String>,
        #[arg(long)]
        mcp_target: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum EventMcpCmd {
    Get,
    Set {
        #[arg(long, value_name = "FILE")]
        from_file: PathBuf,
    },
}

/// `config document` — whole-profile read and secret-preserving write.
#[derive(Debug, Subcommand)]
pub enum DocumentCmd {
    Get,
    /// Replace the profile from JSON `{ "config": ..., "clear_api_key": bool }` on stdin or `--from-file`.
    Set {
        /// JSON file. When omitted, the document is read from stdin.
        #[arg(long, value_name = "FILE")]
        from_file: Option<PathBuf>,
        /// Drop an inline `provider.api_key` even when the body leaves it null.
        #[arg(long)]
        clear_api_key: bool,
    },
}

/// `config secret` — profile `.env` values.
#[derive(Debug, Subcommand)]
pub enum SecretCmd {
    /// Set `KEY` from stdin (a single trailing newline is stripped).
    Set {
        #[arg(long, value_name = "NAME")]
        key: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillCmd {
    List,
    Get {
        name: String,
    },
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        policy: String,
        #[arg(long = "tool", required = true)]
        tools: Vec<String>,
    },
    Set {
        name: String,
        #[arg(long)]
        policy: Option<String>,
        #[arg(long = "tool")]
        tools: Vec<String>,
    },
    Remove {
        name: String,
    },
    /// Replace the catalog from JSON `{ "skills": [...] }` on stdin.
    Replace,
    #[command(subcommand)]
    Tools(SkillToolsCmd),
}

#[derive(Debug, Subcommand)]
pub enum SkillToolsCmd {
    Add { skill: String, tools: Vec<String> },
    Remove { skill: String, tools: Vec<String> },
}

#[derive(Debug, Subcommand)]
pub enum PromptCmd {
    Get,
    Set {
        #[arg(long, conflicts_with_all = ["file", "stdin"])]
        text: Option<String>,
        #[arg(long, value_name = "FILE", conflicts_with_all = ["text", "stdin"])]
        file: Option<PathBuf>,
        /// Read the full prompt from stdin.
        #[arg(long, conflicts_with_all = ["text", "file"])]
        stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RulesCmd {
    Get,
    Set {
        #[arg(long, conflicts_with_all = ["file", "stdin"])]
        text: Option<String>,
        #[arg(long, value_name = "FILE", conflicts_with_all = ["text", "stdin"])]
        file: Option<PathBuf>,
        /// Read the full rules text from stdin.
        #[arg(long, conflicts_with_all = ["text", "file"])]
        stdin: bool,
    },
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum ToolsCmd {
    /// Show effective tool allowlist (access ∩ skills [∩ discovered MCP]).
    Effective {
        /// Connect MCP servers to resolve discovered tools.
        #[arg(long)]
        connect: bool,
    },
}
