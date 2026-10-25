//! Optional progress events from [`crate::agent::AgentEngine`] for ACP streaming.

use std::io::{self, Write};

use serde::Serialize;
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

/// NDJSON progress line for `kbshff run --progress stderr`.
///
/// Stdout of `run` stays a single [`crate::output::RunOutput`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    Thought {
        text: String,
    },
    Message {
        text: String,
    },
    UsageSummary {
        usage: Value,
    },
    ToolStart {
        id: String,
        server: String,
        tool: String,
        arguments: Value,
    },
    ToolFinish {
        id: String,
        ok: bool,
        output: Value,
    },
    Done,
}

impl From<&AgentEvent> for ProgressEvent {
    fn from(event: &AgentEvent) -> Self {
        match event {
            AgentEvent::Thought(text) => Self::Thought { text: text.clone() },
            AgentEvent::Message(text) => Self::Message { text: text.clone() },
            AgentEvent::UsageSummary(usage) => Self::UsageSummary {
                usage: serde_json::to_value(usage).unwrap_or(Value::Null),
            },
            AgentEvent::ToolStart {
                id,
                server,
                tool,
                arguments,
            } => Self::ToolStart {
                id: id.clone(),
                server: server.clone(),
                tool: tool.clone(),
                arguments: arguments.clone(),
            },
            AgentEvent::ToolFinish { id, ok, output } => Self::ToolFinish {
                id: id.clone(),
                ok: *ok,
                output: output.clone(),
            },
        }
    }
}

/// Write one compact JSON progress line to stderr.
pub fn write_progress_line(event: &ProgressEvent) {
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{line}");
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

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn progress_event_is_one_tagged_json_object() {
        let line = serde_json::to_value(ProgressEvent::Message {
            text: "hello".to_string(),
        })
        .expect("json");
        assert_eq!(line["type"], "message");
        assert_eq!(line["text"], "hello");
        let rendered = serde_json::to_string(&ProgressEvent::Done).expect("done");
        assert!(!rendered.contains('\n'));
    }
}
