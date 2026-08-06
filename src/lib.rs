//! Library API for the `agent_Kuibysheff` worker.
//!
//! # Stable surface
//!
//! Downstream library consumers should depend only on:
//!
//! - [`agent`] / [`output`] — run types (`AgentEngine`, `AgentRunRequest`, `RunOutput`, …)
//! - [`config`] / [`limits`] / [`access`] — configuration and policy types
//! - [`tool_api`] / [`tools`] — tool execution traits and policy wrappers
//! - [`event_mcp`] — ordered information-flow hooks backed by MCP tools
//! - [`provider`] — `ModelClient` and chat types (not concrete HTTP adapters)
//! - [`sandbox`] — `SandboxBackend` / `SandboxRunner` abstractions
//! - [`logging`] — `Loggers` and path helpers
//! - [`mcp`] — `McpRegistry`, `McpError` (not OAuth/SSE/HTTP internals)
//!
//! Prefer [`prelude`] for the common stable imports.
//!
//! # Crate-internal
//!
//! Modules marked `pub(crate)` support the CLI composition root ([`app`]) and in-crate
//! unit tests. They are **not** a semver guarantee for external crates. Concrete adapters
//! (clap structs, MCP OAuth/SSE/HTTP, OpenAI HTTP client, logging sink implementations,
//! sandbox mocks) stay `pub(crate)` by default.

#![allow(non_snake_case)]

/// CLI binary composition root (parse → dispatch → wire a run).
///
/// Public so `main.rs` can call it; not part of the stable library facade.
pub mod app;

pub mod access;
pub(crate) mod acp;
pub mod agent;
pub mod billing;
pub mod config;
pub mod event_mcp;
pub mod limits;
pub mod logging;
pub mod mcp;
pub mod output;
pub mod provider;
pub mod sandbox;
pub mod tool_api;
pub mod tools;

pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod context;
pub(crate) mod project_paths;
pub(crate) mod prompt;
pub(crate) mod settings;
pub(crate) mod skills;

/// Common stable re-exports for library consumers and integration tests.
pub mod prelude {
    pub use crate::agent::{AgentEngine, AgentEvent, AgentEventTx, AgentRunRequest, RunCancel};
    pub use crate::event_mcp::{EventStage, PipelineEvents};
    pub use crate::mcp::{Error as McpError, McpRegistry};
    pub use crate::output::{RunOutput, StopReason};
    pub use crate::provider::ModelClient;
    pub use crate::sandbox::{SandboxBackend, SandboxRunner};
    pub use crate::tool_api::ToolExecutor;
}
