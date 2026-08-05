//! Wire and lifecycle types for Event-MCP.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable event stages exposed to Event-MCP handlers by the MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EventStage {
    #[serde(rename = "context.before_model")]
    ContextBeforeModel,
    #[serde(rename = "model.after_response")]
    ModelAfterResponse,
    #[serde(rename = "run.before_output")]
    RunBeforeOutput,
}

impl EventStage {
    /// Returns the stable wire name used in configuration and envelopes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContextBeforeModel => "context.before_model",
            Self::ModelAfterResponse => "model.after_response",
            Self::RunBeforeOutput => "run.before_output",
        }
    }
}

impl fmt::Display for EventStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Versioned arguments sent to an Event-MCP handler through `tools/call`.
#[derive(Debug, Clone, Serialize)]
pub struct EventEnvelope {
    pub spec_version: &'static str,
    pub event_id: String,
    pub event: EventStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    pub payload: Value,
}

impl EventEnvelope {
    pub const SPEC_VERSION: &'static str = "1";

    #[must_use]
    pub fn new(
        event_id: String,
        event: EventStage,
        iteration: Option<u32>,
        payload: Value,
    ) -> Self {
        Self {
            spec_version: Self::SPEC_VERSION,
            event_id,
            event,
            iteration,
            payload,
        }
    }
}

/// Handler-selected action for the current event chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    Pass,
    Replace,
    Reject,
}

/// Strict structured result returned by an Event-MCP handler.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventOutcome {
    pub action: EventAction,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub reason: Option<String>,
}
