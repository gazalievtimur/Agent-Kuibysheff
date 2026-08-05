//! Ordered Event-MCP dispatcher.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::config::{EventFailurePolicy, EventHandlerConfig, EventMcpConfig};
use super::types::{EventAction, EventEnvelope, EventOutcome, EventStage};
use crate::access::QualifiedTool;
use crate::logging::SharedEventSink;
use crate::mcp::{McpError, McpRegistry};
use crate::tool_api::ToolExecutor;

/// Event-MCP dispatch failure surfaced at the agent boundary.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EventMcpError {
    #[error("invalid Event-MCP configuration: {0}")]
    Configuration(String),
    #[error("Event-MCP handler `{handler}` target `{target}` was not discovered")]
    UnknownTarget { handler: String, target: String },
    #[error("Event-MCP handler `{handler}` payload is {actual} bytes; limit is {limit}")]
    PayloadTooLarge {
        handler: String,
        actual: usize,
        limit: usize,
    },
    #[error("Event-MCP handler `{handler}` outcome is {actual} bytes; limit is {limit}")]
    OutcomeTooLarge {
        handler: String,
        actual: usize,
        limit: usize,
    },
    #[error("Event-MCP handler `{handler}` timed out after {timeout_ms} ms")]
    Timeout { handler: String, timeout_ms: u64 },
    #[error("Event-MCP event `{event}` was cancelled")]
    Cancelled { event: EventStage },
    #[error("Event-MCP handler `{handler}` failed: {source}")]
    Mcp {
        handler: String,
        #[source]
        source: McpError,
    },
    #[error("Event-MCP handler `{handler}` returned an invalid outcome: {reason}")]
    InvalidOutcome { handler: String, reason: String },
    #[error("Event-MCP event `{event}` returned an invalid payload: {reason}")]
    InvalidPayload { event: EventStage, reason: String },
    #[error("Event-MCP handler `{handler}` rejected event `{event}`: {reason}")]
    Rejected {
        event: EventStage,
        handler: String,
        reason: String,
    },
}

/// Pipeline extension point used by the agent engine.
#[async_trait]
pub trait PipelineEvents: Send + Sync {
    /// Whether a stage has configured handlers.
    fn has_handlers(&self, event: EventStage) -> bool;

    /// Runs the ordered handler chain and returns the last valid payload.
    async fn dispatch(
        &self,
        event: EventStage,
        payload: Value,
        iteration: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<Value, EventMcpError>;
}

/// No-op implementation used when Event-MCP is not configured.
#[derive(Debug, Default)]
pub struct NoopPipelineEvents;

#[async_trait]
impl PipelineEvents for NoopPipelineEvents {
    fn has_handlers(&self, _event: EventStage) -> bool {
        false
    }

    async fn dispatch(
        &self,
        _event: EventStage,
        payload: Value,
        _iteration: Option<u32>,
        _cancel: &CancellationToken,
    ) -> Result<Value, EventMcpError> {
        Ok(payload)
    }
}

#[async_trait]
trait EventHandlerInvoker: Send + Sync {
    fn available_tools(&self) -> Vec<String>;

    async fn call(&self, server: &str, tool: &str, arguments: Value) -> Result<Value, McpError>;
}

#[async_trait]
impl EventHandlerInvoker for McpRegistry {
    fn available_tools(&self) -> Vec<String> {
        ToolExecutor::available_tools(self)
    }

    async fn call(&self, server: &str, tool: &str, arguments: Value) -> Result<Value, McpError> {
        self.call_event_handler(server, tool, arguments).await
    }
}

#[derive(Debug, Clone)]
struct CompiledHandler {
    id: String,
    server: String,
    tool: String,
    target: String,
    timeout_ms: u64,
    on_error: EventFailurePolicy,
}

struct InvocationAudit<'a> {
    ok: bool,
    action: Option<EventAction>,
    input_bytes: Option<usize>,
    output_bytes: Option<usize>,
    duration_ms: u128,
    error: Option<&'a EventMcpError>,
}

/// Sequential dispatcher for explicitly configured Event-MCP handlers.
pub struct EventMcpDispatcher {
    chains: BTreeMap<EventStage, Vec<CompiledHandler>>,
    invoker: Arc<dyn EventHandlerInvoker>,
    logger: Option<SharedEventSink>,
    max_payload_bytes: usize,
    max_outcome_bytes: usize,
    next_event_id: AtomicU64,
}

impl EventMcpDispatcher {
    /// Compiles handler chains and validates every target against MCP discovery.
    ///
    /// # Errors
    ///
    /// Returns [`EventMcpError::Configuration`] for malformed bindings or
    /// [`EventMcpError::UnknownTarget`] when `tools/list` did not expose a target.
    pub fn new(
        config: &EventMcpConfig,
        registry: Arc<McpRegistry>,
        logger: Option<SharedEventSink>,
    ) -> Result<Self, EventMcpError> {
        Self::new_with_invoker(config, registry, logger)
    }

    fn new_with_invoker(
        config: &EventMcpConfig,
        invoker: Arc<dyn EventHandlerInvoker>,
        logger: Option<SharedEventSink>,
    ) -> Result<Self, EventMcpError> {
        config
            .validate_shape()
            .map_err(EventMcpError::Configuration)?;
        let available: HashSet<_> = invoker.available_tools().into_iter().collect();
        let mut chains = BTreeMap::new();

        for (event, pipeline) in &config.events {
            let mut chain = Vec::with_capacity(pipeline.handlers.len());
            for handler in &pipeline.handlers {
                chain.push(compile_handler(*event, handler, &available)?);
            }
            chains.insert(*event, chain);
        }

        Ok(Self {
            chains,
            invoker,
            logger,
            max_payload_bytes: config.max_payload_bytes,
            max_outcome_bytes: config.max_outcome_bytes,
            next_event_id: AtomicU64::new(1),
        })
    }

    async fn invoke_handler(
        &self,
        event: EventStage,
        handler: &CompiledHandler,
        payload: Value,
        iteration: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<(EventOutcome, usize, usize, u128), EventMcpError> {
        let sequence = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        let envelope = EventEnvelope::new(
            format!("{}:{sequence}", event.as_str()),
            event,
            iteration,
            payload,
        );
        let arguments =
            serde_json::to_value(envelope).map_err(|error| EventMcpError::InvalidOutcome {
                handler: handler.id.clone(),
                reason: format!("failed to encode event envelope: {error}"),
            })?;
        let input_bytes = serde_json::to_vec(&arguments)
            .map_err(|error| EventMcpError::InvalidOutcome {
                handler: handler.id.clone(),
                reason: format!("failed to measure event envelope: {error}"),
            })?
            .len();
        if input_bytes > self.max_payload_bytes {
            return Err(EventMcpError::PayloadTooLarge {
                handler: handler.id.clone(),
                actual: input_bytes,
                limit: self.max_payload_bytes,
            });
        }

        let started = Instant::now();
        let call = self.invoker.call(&handler.server, &handler.tool, arguments);
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Err(EventMcpError::Cancelled { event });
            }
            result = timeout(Duration::from_millis(handler.timeout_ms), call) => result,
        };
        let response = result
            .map_err(|_| EventMcpError::Timeout {
                handler: handler.id.clone(),
                timeout_ms: handler.timeout_ms,
            })?
            .map_err(|source| EventMcpError::Mcp {
                handler: handler.id.clone(),
                source,
            })?;
        let outcome_bytes = serde_json::to_vec(&response)
            .map_err(|error| EventMcpError::InvalidOutcome {
                handler: handler.id.clone(),
                reason: format!("failed to measure MCP result: {error}"),
            })?
            .len();
        if outcome_bytes > self.max_outcome_bytes {
            return Err(EventMcpError::OutcomeTooLarge {
                handler: handler.id.clone(),
                actual: outcome_bytes,
                limit: self.max_outcome_bytes,
            });
        }
        let outcome = parse_outcome(&handler.id, response)?;
        Ok((
            outcome,
            input_bytes,
            outcome_bytes,
            started.elapsed().as_millis(),
        ))
    }

    async fn log_invocation(
        &self,
        event: EventStage,
        handler: &CompiledHandler,
        audit: InvocationAudit<'_>,
    ) {
        let Some(logger) = &self.logger else {
            return;
        };
        let payload = json!({
            "event": event,
            "handler_id": handler.id,
            "target": handler.target,
            "ok": audit.ok,
            "action": audit.action.map(action_name),
            "input_bytes": audit.input_bytes,
            "output_bytes": audit.output_bytes,
            "duration_ms": audit.duration_ms,
            "error": audit.error.map(ToString::to_string),
        });
        if let Err(log_error) = logger.write_event("event_mcp_handler", payload).await {
            warn!(
                event = %event,
                handler = %handler.id,
                error = ?log_error,
                "Event-MCP audit write failed; preserving handler outcome"
            );
        }
    }
}

#[async_trait]
impl PipelineEvents for EventMcpDispatcher {
    fn has_handlers(&self, event: EventStage) -> bool {
        self.chains
            .get(&event)
            .is_some_and(|handlers| !handlers.is_empty())
    }

    async fn dispatch(
        &self,
        event: EventStage,
        payload: Value,
        iteration: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<Value, EventMcpError> {
        let Some(chain) = self.chains.get(&event) else {
            return Ok(payload);
        };
        let mut current = payload;

        for handler in chain {
            let started = Instant::now();
            match self
                .invoke_handler(event, handler, current.clone(), iteration, cancel)
                .await
            {
                Ok((outcome, input_bytes, outcome_bytes, duration_ms)) => {
                    let action = outcome.action;
                    match validate_outcome(event, handler, outcome) {
                        Ok(Some(replacement)) => current = replacement,
                        Ok(None) => {}
                        Err(error) => {
                            self.log_invocation(
                                event,
                                handler,
                                InvocationAudit {
                                    ok: false,
                                    action: Some(action),
                                    input_bytes: Some(input_bytes),
                                    output_bytes: Some(outcome_bytes),
                                    duration_ms,
                                    error: Some(&error),
                                },
                            )
                            .await;
                            if matches!(error, EventMcpError::Rejected { .. })
                                || handler.on_error == EventFailurePolicy::Abort
                            {
                                return Err(error);
                            }
                            warn!(
                                event = %event,
                                handler = %handler.id,
                                error = %error,
                                "Event-MCP outcome failed validation with continue policy"
                            );
                            continue;
                        }
                    }
                    self.log_invocation(
                        event,
                        handler,
                        InvocationAudit {
                            ok: true,
                            action: Some(action),
                            input_bytes: Some(input_bytes),
                            output_bytes: Some(outcome_bytes),
                            duration_ms,
                            error: None,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    self.log_invocation(
                        event,
                        handler,
                        InvocationAudit {
                            ok: false,
                            action: None,
                            input_bytes: None,
                            output_bytes: None,
                            duration_ms: started.elapsed().as_millis(),
                            error: Some(&error),
                        },
                    )
                    .await;
                    if matches!(error, EventMcpError::Cancelled { .. })
                        || handler.on_error == EventFailurePolicy::Abort
                    {
                        return Err(error);
                    }
                    warn!(
                        event = %event,
                        handler = %handler.id,
                        error = %error,
                        "Event-MCP handler failed with continue policy"
                    );
                }
            }
        }
        Ok(current)
    }
}

fn compile_handler(
    event: EventStage,
    handler: &EventHandlerConfig,
    available: &HashSet<String>,
) -> Result<CompiledHandler, EventMcpError> {
    let target = handler.target.trim();
    let qualified = QualifiedTool::parse(target).map_err(|reason| {
        EventMcpError::Configuration(format!(
            "event_mcp handler `{}` for event `{event}` has invalid target `{target}`: {reason}",
            handler.id
        ))
    })?;
    if !available.contains(target) {
        return Err(EventMcpError::UnknownTarget {
            handler: handler.id.clone(),
            target: target.to_string(),
        });
    }
    Ok(CompiledHandler {
        id: handler.id.trim().to_string(),
        server: qualified.server().to_string(),
        tool: qualified.tool().to_string(),
        target: target.to_string(),
        timeout_ms: handler.timeout_ms,
        on_error: handler.on_error,
    })
}

fn parse_outcome(handler: &str, response: Value) -> Result<EventOutcome, EventMcpError> {
    if response
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(EventMcpError::InvalidOutcome {
            handler: handler.to_string(),
            reason: "MCP CallToolResult set isError=true".to_string(),
        });
    }

    let outcome_value = if let Some(structured) = response.get("structuredContent") {
        structured.clone()
    } else {
        let content = response
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| EventMcpError::InvalidOutcome {
                handler: handler.to_string(),
                reason: "expected structuredContent or one JSON text content item".to_string(),
            })?;
        let mut texts = content.iter().filter_map(|item| {
            (item.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| item.get("text").and_then(Value::as_str))
                .flatten()
        });
        let text = texts.next().ok_or_else(|| EventMcpError::InvalidOutcome {
            handler: handler.to_string(),
            reason: "compatibility result has no text content".to_string(),
        })?;
        if texts.next().is_some() {
            return Err(EventMcpError::InvalidOutcome {
                handler: handler.to_string(),
                reason: "compatibility result must contain exactly one text item".to_string(),
            });
        }
        serde_json::from_str(text).map_err(|error| EventMcpError::InvalidOutcome {
            handler: handler.to_string(),
            reason: format!("text content is not one JSON object: {error}"),
        })?
    };

    serde_json::from_value(outcome_value).map_err(|error| EventMcpError::InvalidOutcome {
        handler: handler.to_string(),
        reason: error.to_string(),
    })
}

fn validate_outcome(
    event: EventStage,
    handler: &CompiledHandler,
    outcome: EventOutcome,
) -> Result<Option<Value>, EventMcpError> {
    match outcome.action {
        EventAction::Pass if outcome.payload.is_none() && outcome.reason.is_none() => Ok(None),
        EventAction::Replace if outcome.reason.is_none() => {
            outcome
                .payload
                .map(Some)
                .ok_or_else(|| EventMcpError::InvalidOutcome {
                    handler: handler.id.clone(),
                    reason: "replace requires payload".to_string(),
                })
        }
        EventAction::Reject if outcome.payload.is_none() => {
            let reason = outcome
                .reason
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "handler rejected the event".to_string());
            Err(EventMcpError::Rejected {
                event,
                handler: handler.id.clone(),
                reason,
            })
        }
        EventAction::Pass => Err(EventMcpError::InvalidOutcome {
            handler: handler.id.clone(),
            reason: "pass must not include payload or reason".to_string(),
        }),
        EventAction::Replace => Err(EventMcpError::InvalidOutcome {
            handler: handler.id.clone(),
            reason: "replace must include payload and must not include reason".to_string(),
        }),
        EventAction::Reject => Err(EventMcpError::InvalidOutcome {
            handler: handler.id.clone(),
            reason: "reject must not include payload".to_string(),
        }),
    }
}

const fn action_name(action: EventAction) -> &'static str {
    match action {
        EventAction::Pass => "pass",
        EventAction::Replace => "replace",
        EventAction::Reject => "reject",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::event_mcp::config::{EventHandlerConfig, EventPipelineConfig};

    struct StubInvoker {
        tools: Vec<String>,
        responses: Mutex<VecDeque<Result<Value, McpError>>>,
        calls: Mutex<Vec<(String, String, Value)>>,
        delay: Option<Duration>,
    }

    #[async_trait]
    impl EventHandlerInvoker for StubInvoker {
        fn available_tools(&self) -> Vec<String> {
            self.tools.clone()
        }

        async fn call(
            &self,
            server: &str,
            tool: &str,
            arguments: Value,
        ) -> Result<Value, McpError> {
            self.calls.lock().expect("calls").push((
                server.to_string(),
                tool.to_string(),
                arguments,
            ));
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            self.responses
                .lock()
                .expect("responses")
                .pop_front()
                .expect("stub response")
        }
    }

    fn handler(id: &str, target: &str, on_error: EventFailurePolicy) -> EventHandlerConfig {
        EventHandlerConfig {
            id: id.to_string(),
            target: target.to_string(),
            timeout_ms: 1_000,
            on_error,
        }
    }

    fn config(handlers: Vec<EventHandlerConfig>) -> EventMcpConfig {
        EventMcpConfig {
            events: BTreeMap::from([(
                EventStage::ContextBeforeModel,
                EventPipelineConfig { handlers },
            )]),
            ..EventMcpConfig::default()
        }
    }

    fn structured(action: &str, payload: Option<Value>) -> Value {
        let mut outcome = json!({ "action": action });
        if let Some(payload) = payload {
            outcome["payload"] = payload;
        }
        json!({ "structuredContent": outcome })
    }

    #[test]
    fn single_json_text_content_is_supported_as_compatibility_fallback() {
        let outcome = parse_outcome(
            "legacy",
            json!({
                "content": [{
                    "type": "text",
                    "text": "{\"action\":\"replace\",\"payload\":{\"ok\":true}}"
                }]
            }),
        )
        .expect("parse fallback");

        assert_eq!(outcome.action, EventAction::Replace);
        assert_eq!(outcome.payload, Some(json!({"ok": true})));
    }

    #[tokio::test]
    async fn multiple_servers_run_in_configured_order_and_chain_replacements() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["beta.second".to_string(), "alpha.first".to_string()],
            responses: Mutex::new(VecDeque::from([
                Ok(structured("replace", Some(json!({"value": 2})))),
                Ok(structured("replace", Some(json!({"value": 3})))),
            ])),
            calls: Mutex::new(Vec::new()),
            delay: None,
        });
        let dispatcher = EventMcpDispatcher::new_with_invoker(
            &config(vec![
                handler("first", "alpha.first", EventFailurePolicy::Abort),
                handler("second", "beta.second", EventFailurePolicy::Abort),
            ]),
            invoker.clone(),
            None,
        )
        .expect("dispatcher");

        let output = dispatcher
            .dispatch(
                EventStage::ContextBeforeModel,
                json!({"value": 1}),
                Some(4),
                &CancellationToken::new(),
            )
            .await
            .expect("dispatch");

        assert_eq!(output, json!({"value": 3}));
        let calls = invoker.calls.lock().expect("calls");
        assert_eq!(calls[0].0, "alpha");
        assert_eq!(calls[1].0, "beta");
        assert_eq!(calls[0].2["payload"], json!({"value": 1}));
        assert_eq!(calls[1].2["payload"], json!({"value": 2}));
    }

    #[tokio::test]
    async fn continue_policy_keeps_last_valid_payload() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["alpha.first".to_string(), "beta.second".to_string()],
            responses: Mutex::new(VecDeque::from([
                Err(McpError::Protocol {
                    server: "alpha".to_string(),
                    error: "broken".to_string(),
                }),
                Ok(structured("pass", None)),
            ])),
            calls: Mutex::new(Vec::new()),
            delay: None,
        });
        let dispatcher = EventMcpDispatcher::new_with_invoker(
            &config(vec![
                handler("first", "alpha.first", EventFailurePolicy::Continue),
                handler("second", "beta.second", EventFailurePolicy::Abort),
            ]),
            invoker.clone(),
            None,
        )
        .expect("dispatcher");

        let initial = json!({"value": 1});
        let output = dispatcher
            .dispatch(
                EventStage::ContextBeforeModel,
                initial.clone(),
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("dispatch");

        assert_eq!(output, initial);
        assert_eq!(invoker.calls.lock().expect("calls").len(), 2);
    }

    #[tokio::test]
    async fn malformed_outcome_uses_continue_policy() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["alpha.first".to_string(), "beta.second".to_string()],
            responses: Mutex::new(VecDeque::from([
                Ok(json!({"structuredContent": {"action": "replace"}})),
                Ok(structured("pass", None)),
            ])),
            calls: Mutex::new(Vec::new()),
            delay: None,
        });
        let dispatcher = EventMcpDispatcher::new_with_invoker(
            &config(vec![
                handler("first", "alpha.first", EventFailurePolicy::Continue),
                handler("second", "beta.second", EventFailurePolicy::Abort),
            ]),
            invoker.clone(),
            None,
        )
        .expect("dispatcher");

        let initial = json!({"value": 1});
        let output = dispatcher
            .dispatch(
                EventStage::ContextBeforeModel,
                initial.clone(),
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("dispatch");

        assert_eq!(output, initial);
        assert_eq!(invoker.calls.lock().expect("calls").len(), 2);
    }

    #[tokio::test]
    async fn reject_always_stops_the_chain() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["alpha.first".to_string(), "beta.second".to_string()],
            responses: Mutex::new(VecDeque::from([
                Ok(json!({
                    "structuredContent": {
                        "action": "reject",
                        "reason": "unsafe"
                    }
                })),
                Ok(structured("pass", None)),
            ])),
            calls: Mutex::new(Vec::new()),
            delay: None,
        });
        let dispatcher = EventMcpDispatcher::new_with_invoker(
            &config(vec![
                handler("first", "alpha.first", EventFailurePolicy::Continue),
                handler("second", "beta.second", EventFailurePolicy::Abort),
            ]),
            invoker.clone(),
            None,
        )
        .expect("dispatcher");

        let error = dispatcher
            .dispatch(
                EventStage::ContextBeforeModel,
                Value::Null,
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("rejected");

        assert!(matches!(error, EventMcpError::Rejected { .. }));
        assert_eq!(invoker.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn timeout_aborts_when_configured() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["alpha.first".to_string()],
            responses: Mutex::new(VecDeque::from([Ok(structured("pass", None))])),
            calls: Mutex::new(Vec::new()),
            delay: Some(Duration::from_millis(50)),
        });
        let mut timed_handler = handler("first", "alpha.first", EventFailurePolicy::Abort);
        timed_handler.timeout_ms = 1;
        let dispatcher =
            EventMcpDispatcher::new_with_invoker(&config(vec![timed_handler]), invoker, None)
                .expect("dispatcher");

        let error = dispatcher
            .dispatch(
                EventStage::ContextBeforeModel,
                Value::Null,
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("timeout");

        assert!(matches!(error, EventMcpError::Timeout { .. }));
    }

    #[tokio::test]
    async fn different_events_keep_independent_chains() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["alpha.context".to_string(), "beta.response".to_string()],
            responses: Mutex::new(VecDeque::from([
                Ok(structured("pass", None)),
                Ok(structured("pass", None)),
            ])),
            calls: Mutex::new(Vec::new()),
            delay: None,
        });
        let event_config = EventMcpConfig {
            events: BTreeMap::from([
                (
                    EventStage::ContextBeforeModel,
                    EventPipelineConfig {
                        handlers: vec![handler(
                            "context",
                            "alpha.context",
                            EventFailurePolicy::Abort,
                        )],
                    },
                ),
                (
                    EventStage::ModelAfterResponse,
                    EventPipelineConfig {
                        handlers: vec![handler(
                            "response",
                            "beta.response",
                            EventFailurePolicy::Abort,
                        )],
                    },
                ),
            ]),
            ..EventMcpConfig::default()
        };
        let dispatcher = EventMcpDispatcher::new_with_invoker(&event_config, invoker.clone(), None)
            .expect("dispatcher");

        dispatcher
            .dispatch(
                EventStage::ContextBeforeModel,
                Value::Null,
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("context");
        dispatcher
            .dispatch(
                EventStage::ModelAfterResponse,
                Value::Null,
                Some(1),
                &CancellationToken::new(),
            )
            .await
            .expect("response");

        let calls = invoker.calls.lock().expect("calls");
        assert_eq!(calls[0].0, "alpha");
        assert_eq!(calls[1].0, "beta");
    }

    #[test]
    fn target_must_be_discovered() {
        let invoker = Arc::new(StubInvoker {
            tools: Vec::new(),
            responses: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            delay: None,
        });

        let error = EventMcpDispatcher::new_with_invoker(
            &config(vec![handler(
                "missing",
                "server.tool",
                EventFailurePolicy::Abort,
            )]),
            invoker,
            None,
        )
        .err()
        .expect("unknown target");

        assert!(matches!(error, EventMcpError::UnknownTarget { .. }));
    }

    #[tokio::test]
    async fn cancellation_always_aborts_continue_handler() {
        let invoker = Arc::new(StubInvoker {
            tools: vec!["alpha.first".to_string()],
            responses: Mutex::new(VecDeque::from([Ok(structured("pass", None))])),
            calls: Mutex::new(Vec::new()),
            delay: Some(Duration::from_secs(30)),
        });
        let dispatcher = EventMcpDispatcher::new_with_invoker(
            &config(vec![handler(
                "first",
                "alpha.first",
                EventFailurePolicy::Continue,
            )]),
            invoker,
            None,
        )
        .expect("dispatcher");
        let cancel = CancellationToken::new();
        cancel.cancel();

        let error = dispatcher
            .dispatch(EventStage::ContextBeforeModel, Value::Null, None, &cancel)
            .await
            .expect_err("cancelled");

        assert!(matches!(error, EventMcpError::Cancelled { .. }));
    }
}
