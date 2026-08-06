//! Optional progress events from [`crate::agent::AgentEngine`] for ACP streaming.

use serde_json::Value;
use tokio::sync::mpsc;

use crate::output::UsageReport;

/// Progress event emitted during an agent turn.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Model thought / reasoning text.
    Thought(String),
    /// Final or intermediate user-visible message text.
    Message(String),
    /// Host-owned token and monetary usage summary for the completed turn.
    UsageSummary(UsageReport),
    /// A tool invocation is about to start.
    ToolStart {
        id: String,
        server: String,
        tool: String,
        arguments: Value,
    },
    /// A tool invocation finished (success or failure payload in `output`).
    ToolFinish { id: String, ok: bool, output: Value },
}

/// Fire-and-forget sender for [`AgentEvent`]s. Dropped / closed receivers are ignored.
#[derive(Clone, Default)]
pub struct AgentEventTx {
    inner: Option<mpsc::UnboundedSender<AgentEvent>>,
}

impl AgentEventTx {
    /// Creates a no-op sink (CLI `run` path).
    #[must_use]
    pub fn noop() -> Self {
        Self { inner: None }
    }

    /// Creates a sink that forwards to `tx`.
    #[must_use]
    pub fn from_sender(tx: mpsc::UnboundedSender<AgentEvent>) -> Self {
        Self { inner: Some(tx) }
    }

    /// Emits an event when a receiver is still connected.
    pub fn emit(&self, event: AgentEvent) {
        if let Some(tx) = &self.inner {
            let _ = tx.send(event);
        }
    }
}
