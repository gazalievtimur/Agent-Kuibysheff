//! Agent run types and engine.
//!
//! Stable: [`AgentEngine`], [`AgentRunRequest`], [`AgentError`], [`RunCancel`], [`AgentEvent`].

pub(crate) mod events;
pub(crate) mod r#loop;
pub(crate) mod run_cancel;

pub use events::{AgentEvent, AgentEventTx};
pub use r#loop::{AgentEngine, AgentError, AgentRunRequest};
pub use run_cancel::RunCancel;
