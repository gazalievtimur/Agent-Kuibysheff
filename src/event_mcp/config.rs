//! Declarative Event-MCP pipeline configuration.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use super::EventStage;

/// Event-MCP configuration. An empty value is a no-op.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EventMcpConfig {
    pub events: BTreeMap<EventStage, EventPipelineConfig>,
    #[serde(default = "EventMcpConfig::default_max_payload_bytes")]
    pub max_payload_bytes: usize,
    #[serde(default = "EventMcpConfig::default_max_outcome_bytes")]
    pub max_outcome_bytes: usize,
}

impl Default for EventMcpConfig {
    fn default() -> Self {
        Self {
            events: BTreeMap::new(),
            max_payload_bytes: Self::default_max_payload_bytes(),
            max_outcome_bytes: Self::default_max_outcome_bytes(),
        }
    }
}

impl EventMcpConfig {
    #[must_use]
    pub const fn default_max_payload_bytes() -> usize {
        1_048_576
    }

    #[must_use]
    pub const fn default_max_outcome_bytes() -> usize {
        1_048_576
    }

    /// Validates configuration properties that do not require MCP discovery.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason for an invalid limit, id, target, or timeout.
    pub fn validate_shape(&self) -> Result<(), String> {
        if self.max_payload_bytes == 0 {
            return Err("event_mcp.max_payload_bytes must be greater than zero".to_string());
        }
        if self.max_outcome_bytes == 0 {
            return Err("event_mcp.max_outcome_bytes must be greater than zero".to_string());
        }

        for (event, pipeline) in &self.events {
            let mut ids = HashSet::with_capacity(pipeline.handlers.len());
            for handler in &pipeline.handlers {
                let id = handler.id.trim();
                if id.is_empty() {
                    return Err(format!("event_mcp event `{event}` has an empty handler id"));
                }
                if !ids.insert(id) {
                    return Err(format!(
                        "event_mcp event `{event}` has duplicate handler id `{id}`"
                    ));
                }
                if handler.target.trim().is_empty() {
                    return Err(format!(
                        "event_mcp handler `{id}` for event `{event}` has an empty target"
                    ));
                }
                if handler.timeout_ms == 0 {
                    return Err(format!(
                        "event_mcp handler `{id}` for event `{event}` must have timeout_ms greater than zero"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Ordered handlers subscribed to one event.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventPipelineConfig {
    #[serde(default)]
    pub handlers: Vec<EventHandlerConfig>,
}

/// One MCP tool bound as an event handler.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventHandlerConfig {
    pub id: String,
    pub target: String,
    #[serde(default = "EventHandlerConfig::default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub on_error: EventFailurePolicy,
}

impl EventHandlerConfig {
    #[must_use]
    pub const fn default_timeout_ms() -> u64 {
        5_000
    }
}

/// Technical failure policy for an individual handler.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventFailurePolicy {
    Continue,
    #[default]
    Abort,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_configuration_is_empty() {
        let config: EventMcpConfig = serde_yaml::from_str("{}").expect("parse");

        assert!(config.events.is_empty());
        assert_eq!(
            config.max_payload_bytes,
            EventMcpConfig::default_max_payload_bytes()
        );
    }

    #[test]
    fn handlers_preserve_declaration_order() {
        let config: EventMcpConfig = serde_yaml::from_str(
            r#"
events:
  context.before_model:
    handlers:
      - id: first
        target: alpha.redact
      - id: second
        target: beta.compact
        on_error: continue
"#,
        )
        .expect("parse");

        let handlers = &config.events[&EventStage::ContextBeforeModel].handlers;
        assert_eq!(handlers[0].id, "first");
        assert_eq!(handlers[1].id, "second");
        assert_eq!(handlers[1].on_error, EventFailurePolicy::Continue);
        config.validate_shape().expect("valid shape");
    }

    #[test]
    fn duplicate_handler_ids_are_rejected_per_event() {
        let config: EventMcpConfig = serde_yaml::from_str(
            r#"
events:
  model.after_response:
    handlers:
      - id: validate
        target: first.validate
      - id: validate
        target: second.validate
"#,
        )
        .expect("parse");

        let error = config.validate_shape().expect_err("duplicate id");
        assert!(error.contains("duplicate handler id `validate`"));
    }

    #[test]
    fn unknown_stage_is_rejected() {
        let error = serde_yaml::from_str::<EventMcpConfig>(
            r#"
events:
  model.unknown:
    handlers: []
"#,
        )
        .expect_err("unknown stage");

        assert!(error.to_string().contains("unknown variant"));
    }
}
