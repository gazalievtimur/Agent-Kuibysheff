//! Event-driven MCP middleware for agent information-flow stages.
//!
//! Event handlers remain ordinary MCP tools. [`EventMcpDispatcher`] invokes explicitly
//! configured handlers in deterministic order and validates their versioned outcomes.

mod config;
mod dispatcher;
mod types;

pub use config::{EventFailurePolicy, EventHandlerConfig, EventMcpConfig, EventPipelineConfig};
pub use dispatcher::{EventMcpDispatcher, EventMcpError, NoopPipelineEvents, PipelineEvents};
pub use types::{EventAction, EventEnvelope, EventOutcome, EventStage};
