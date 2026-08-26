//! Agent run types and engine.
//!
//! Stable: [`AgentEngine`], [`AgentRunRequest`], [`AgentError`], [`RunCancel`], [`AgentEvent`].

pub(crate) mod events;
pub(crate) mod r#loop;
pub(crate) mod run_cancel;

use std::time::{SystemTime, UNIX_EPOCH};

pub use events::{AgentEvent, AgentEventTx};
pub use r#loop::{AgentEngine, AgentError, AgentRunRequest};
pub use run_cancel::RunCancel;

/// Generates a unique run id (`run-{timestamp}-{rand}`).
#[must_use]
pub fn generate_run_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("run-{timestamp:032x}-{:016x}", rand::random::<u64>())
}
