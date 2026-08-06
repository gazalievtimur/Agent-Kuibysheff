use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{info, instrument, warn};

use super::directive::{approx_json_object_count, content_preview, parse_directive};
use super::history::{prune_message_history, push_message};
use crate::access::QualifiedTool;
use crate::agent::{AgentEvent, AgentEventTx, RunCancel};
use crate::billing::{BillingError, CostResolverChain, RunCostReport, RunCostTracker};
use crate::config::ProviderHistoryConfig;
use crate::event_mcp::{EventMcpError, EventStage, NoopPipelineEvents, PipelineEvents};
use crate::limits::{LimitExceeded, LimitsConfig, RunMetrics};
use crate::logging::{Loggers, LoggingError};
use crate::output::{RunOutput, StopReason, UsageReport};
use crate::provider::{ChatMessage, ChatRole, ModelClient};
use crate::tool_api::ToolExecutor;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider failure: {0}")]
    Provider(#[from] crate::provider::Error),
    #[error("tool failure: {0}")]
    Tool(#[from] crate::tool_api::ToolError),
    #[error("failed to decode model directive: {0}")]
    DirectiveDecode(#[from] serde_json::Error),
    #[error("internal logging failure: {0}")]
    Logging(#[from] LoggingError),
    #[error("Event-MCP failure: {0}")]
    EventMcp(#[from] EventMcpError),
    #[error("billing failure: {0}")]
    Billing(#[from] BillingError),
}

#[derive(Clone)]
pub struct AgentRunRequest {
    pub prompt: String,
    pub system_prompt: String,
    pub input_files_context: String,
    pub limits: LimitsConfig,
    /// Model context-window pruning budgets (`provider.history`).
    pub history: ProviderHistoryConfig,
    /// Cooperative cancel + wall-clock deadline for this run.
    pub cancel: RunCancel,
    /// Optional progress sink (ACP streaming); no-op for CLI `run`.
    pub events: AgentEventTx,
}

pub struct AgentEngine {
    model: Arc<dyn ModelClient>,
    tools: Arc<dyn ToolExecutor>,
    pipeline_events: Arc<dyn PipelineEvents>,
    billing_resolver: Arc<CostResolverChain>,
    billing_currency: String,
    run_id: String,
    loggers: Loggers,
}

impl AgentEngine {
    pub fn new(
        model: Arc<dyn ModelClient>,
        tools: Arc<dyn ToolExecutor>,
        loggers: Loggers,
    ) -> Self {
        Self {
            model,
            tools,
            pipeline_events: Arc::new(NoopPipelineEvents),
            billing_resolver: Arc::new(CostResolverChain::default()),
            billing_currency: "USD".to_string(),
            run_id: generate_run_id(),
            loggers,
        }
    }

    /// Attaches an information-flow event pipeline to this engine.
    #[must_use]
    pub fn with_pipeline_events(mut self, pipeline_events: Arc<dyn PipelineEvents>) -> Self {
        self.pipeline_events = pipeline_events;
        self
    }

    /// Attaches the host-owned cost resolver and run identity.
    #[must_use]
    pub fn with_billing(
        mut self,
        resolver: Arc<CostResolverChain>,
        currency: impl Into<String>,
        run_id: impl Into<String>,
    ) -> Self {
        self.billing_resolver = resolver;
        self.billing_currency = currency.into();
        self.run_id = run_id.into();
        self
    }

    pub async fn run(&self, request: AgentRunRequest) -> RunOutput {
        let result = self.run_inner(request).await;
        self.loggers.shutdown().await;
        match result {
            Ok(out) => out,
            Err((err, usage)) => RunOutput {
                run_id: self.run_id.clone(),
                result: err.to_string(),
                usage,
                stop_reason: StopReason::Error,
                logs: self.loggers.report(),
            },
        }
    }

    #[instrument(skip(self, request), fields(prompt_len = request.prompt.len()))]
    #[allow(clippy::too_many_lines)]
    async fn run_inner(
        &self,
        request: AgentRunRequest,
    ) -> Result<RunOutput, (AgentError, UsageReport)> {
        let AgentRunRequest {
            prompt,
            system_prompt,
            input_files_context,
            limits,
            history,
            cancel,
            events,
        } = request;

        let available_tools = self.tools.available_tools();
        let user_message = build_user_message(&prompt, &input_files_context, &available_tools);
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, system_prompt),
            ChatMessage::new(ChatRole::User, user_message),
        ];
        let mut full_history = messages.clone();
        let mut metrics = RunMetrics::new();
        let mut cost_tracker =
            RunCostTracker::new(self.billing_currency.clone(), limits.max_cost.clone()).map_err(
                |error| {
                    (
                        error.into(),
                        build_usage_report(&metrics, RunCostReport::default()),
                    )
                },
            )?;
        let mut final_result = String::new();
        let mut stop_reason = StopReason::LimitReached;
        let mut diag = RunDiagnostics::default();

        // Align hard deadline with RunMetrics wall clock (not composition-root setup time).
        cancel.arm_deadline(Duration::from_secs(limits.max_duration_sec));

        loop {
            if should_stop_for_duration(&metrics, &limits, &mut final_result, &mut stop_reason) {
                break;
            }
            if should_stop_for_cancel(
                &cancel,
                &metrics,
                &limits,
                &mut final_result,
                &mut stop_reason,
            ) {
                break;
            }
            if cost_tracker.limit_hit() {
                final_result = "Execution stopped due to limit: max_cost".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }
            match metrics.pre_step_check(&limits) {
                Ok(()) => {}
                Err(limit) => {
                    final_result = format!("Execution stopped due to limit: {}", limit_name(limit));
                    stop_reason = StopReason::LimitReached;
                    break;
                }
            }
            metrics.begin_iteration();
            let iteration = metrics.iterations();

            let provider_snapshot = if self
                .pipeline_events
                .has_handlers(EventStage::ContextBeforeModel)
            {
                let transformed = self
                    .pipeline_events
                    .dispatch(
                        EventStage::ContextBeforeModel,
                        json!({ "messages": &messages }),
                        Some(iteration),
                        cancel.token(),
                    )
                    .await;
                let transformed = match transformed {
                    Ok(payload) => payload,
                    Err(err) => {
                        self.loggers.persist_chat_history(&full_history, None).await;
                        return Err((
                            err.into(),
                            build_usage_report(&metrics, cost_tracker.report()),
                        ));
                    }
                };
                match decode_context_before_model(&messages, transformed) {
                    Ok(snapshot) => Some(snapshot),
                    Err(err) => {
                        self.loggers.persist_chat_history(&full_history, None).await;
                        return Err((
                            err.into(),
                            build_usage_report(&metrics, cost_tracker.report()),
                        ));
                    }
                }
            } else {
                None
            };
            let provider_messages = provider_snapshot.as_deref().unwrap_or(&messages);
            let accounted = tokio::select! {
                biased;
                () = cancel.token().cancelled() => {
                    set_cancel_or_duration_stop(
                        &cancel,
                        &metrics,
                        &limits,
                        &mut final_result,
                        &mut stop_reason,
                    );
                    break;
                }
                completion = self.model.complete_accounted(provider_messages) => completion,
            };
            let (mut completion, attempts) = match accounted {
                Ok(accounted) => (accounted.response, accounted.attempts),
                Err(failure) => {
                    let cost_start = cost_tracker.report().requests.len();
                    for attempt in &failure.attempts {
                        let resolution = self.billing_resolver.resolve(attempt).await;
                        cost_tracker.record(iteration, attempt, resolution);
                    }
                    let report = cost_tracker.report();
                    let attempt_costs = &report.requests[cost_start..];
                    if let Some(ai_log) = &self.loggers.ai {
                        if let Err(error) = ai_log
                            .write_event(
                                "ai_provider_failure",
                                json!({
                                    "run_id": self.run_id,
                                    "iteration": iteration,
                                    "attempts": failure.attempts,
                                    "costs": attempt_costs,
                                }),
                            )
                            .await
                        {
                            warn!(iteration, error = ?error, "AI provider failure audit write failed");
                        }
                    }
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err((failure.error.into(), build_usage_report(&metrics, report)));
                }
            };
            let cost_start = cost_tracker.report().requests.len();
            for attempt in &attempts {
                let resolution = self.billing_resolver.resolve(attempt).await;
                cost_tracker.record(iteration, attempt, resolution);
            }
            metrics.add_tokens(completion.usage);
            if self
                .pipeline_events
                .has_handlers(EventStage::ModelAfterResponse)
            {
                let transformed = self
                    .pipeline_events
                    .dispatch(
                        EventStage::ModelAfterResponse,
                        json!({ "content": completion.content }),
                        Some(iteration),
                        cancel.token(),
                    )
                    .await;
                let transformed = match transformed {
                    Ok(payload) => payload,
                    Err(err) => {
                        self.loggers.persist_chat_history(&full_history, None).await;
                        return Err((
                            err.into(),
                            build_usage_report(&metrics, cost_tracker.report()),
                        ));
                    }
                };
                completion.content = match decode_model_after_response(transformed) {
                    Ok(content) => content,
                    Err(err) => {
                        self.loggers.persist_chat_history(&full_history, None).await;
                        return Err((
                            err.into(),
                            build_usage_report(&metrics, cost_tracker.report()),
                        ));
                    }
                };
            }
            let current_cost_report = cost_tracker.report();
            let attempt_costs = &current_cost_report.requests[cost_start..];
            if let Some(ai_log) = &self.loggers.ai {
                if let Err(err) = ai_log
                    .write_event(
                        "ai_completion",
                        json!({
                            "run_id": self.run_id,
                            "iteration": iteration,
                            "content": completion.content,
                            "usage": completion.usage,
                            "provider_attempts": attempts,
                            "costs": attempt_costs,
                        }),
                    )
                    .await
                {
                    warn!(
                        iteration,
                        error = ?err,
                        "AI audit log write failed after successful completion; continuing run"
                    );
                }
            }
            if metrics.tokens_limit_hit(&limits) {
                final_result = "Execution stopped due to limit: max_tokens".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }
            if cost_tracker.limit_hit() {
                final_result = "Execution stopped due to limit: max_cost".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }
            if should_stop_for_duration(&metrics, &limits, &mut final_result, &mut stop_reason) {
                break;
            }
            if should_stop_for_cancel(
                &cancel,
                &metrics,
                &limits,
                &mut final_result,
                &mut stop_reason,
            ) {
                break;
            }

            let directive = match parse_directive(&completion.content) {
                Ok(v) => v,
                Err(err) => {
                    diag.parse_failures = diag.parse_failures.saturating_add(1);
                    let content_len = completion.content.len();
                    let approx_json_objects = approx_json_object_count(&completion.content);
                    let preview = content_preview(&completion.content, 200);
                    warn!(
                        iteration,
                        error = %err,
                        content_len,
                        approx_json_objects,
                        preview = %preview,
                        "directive parse failed; tool calls from this turn were not executed"
                    );
                    if let Some(ai_log) = &self.loggers.ai {
                        if let Err(log_err) = ai_log
                            .write_event(
                                "directive_parse_failed",
                                json!({
                                    "iteration": iteration,
                                    "error": err.to_string(),
                                    "content_len": content_len,
                                    "approx_json_objects": approx_json_objects,
                                    "preview": preview,
                                }),
                            )
                            .await
                        {
                            warn!(
                                iteration,
                                error = ?log_err,
                                "AI audit log write failed for directive_parse_failed; continuing run"
                            );
                        }
                    }
                    push_message(
                        &mut messages,
                        &mut full_history,
                        ChatMessage::new(ChatRole::Assistant, completion.content),
                        &history,
                    );
                    push_message(
                        &mut messages,
                        &mut full_history,
                        ChatMessage::new(
                            ChatRole::User,
                            json!({
                                "parse_error": err.to_string(),
                                "hint": "Respond with exactly one JSON object only. Previous turn was not executed. No markdown fences. Required shape: {\"done\": bool, \"thought\": string, \"tool_calls\": [...], \"result\": string|null}"
                            })
                            .to_string(),
                        ),
                        &history,
                    );
                    prune_message_history(&mut messages, &history);
                    continue;
                }
            };

            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::Assistant, completion.content),
                &history,
            );

            if let Some(thought) = directive.thought.as_deref() {
                if !thought.trim().is_empty() {
                    events.emit(AgentEvent::Thought(thought.to_string()));
                }
            }

            for (tool_index, tool_call) in directive.tool_calls.into_iter().enumerate() {
                let tool_call_id = format!("tc-{iteration}-{tool_index}");
                let qualified =
                    match QualifiedTool::parse(&format!("{}.{}", tool_call.server, tool_call.tool))
                    {
                        Ok(qualified) => qualified,
                        Err(reason) => {
                            warn!(
                                iteration,
                                server = %tool_call.server,
                                tool = %tool_call.tool,
                                error = %reason,
                                "tool call name rejected; returning error to the model"
                            );
                            self.log_tool_event(
                                "tool_call_failed",
                                json!({
                                    "iteration": iteration,
                                    "server": tool_call.server,
                                    "tool": tool_call.tool,
                                    "ok": false,
                                    "error": reason,
                                }),
                            )
                            .await;
                            events.emit(AgentEvent::ToolStart {
                                id: tool_call_id.clone(),
                                server: tool_call.server.clone(),
                                tool: tool_call.tool.clone(),
                                arguments: tool_call.arguments.clone(),
                            });
                            events.emit(AgentEvent::ToolFinish {
                                id: tool_call_id,
                                ok: false,
                                output: json!({ "error": reason }),
                            });
                            push_message(
                                &mut messages,
                                &mut full_history,
                                ChatMessage::new(
                                    ChatRole::User,
                                    json!({
                                        "tool_result": {
                                            "server": tool_call.server,
                                            "tool": tool_call.tool,
                                            "error": reason
                                        }
                                    })
                                    .to_string(),
                                ),
                                &history,
                            );
                            prune_message_history(&mut messages, &history);
                            continue;
                        }
                    };
                let server = qualified.server().to_string();
                let tool = qualified.tool().to_string();
                let qualified_tool = qualified.qualified();

                self.log_tool_event(
                    "tool_call_started",
                    json!({
                        "iteration": iteration,
                        "server": server,
                        "tool": tool,
                    }),
                )
                .await;

                events.emit(AgentEvent::ToolStart {
                    id: tool_call_id.clone(),
                    server: server.clone(),
                    tool: tool.clone(),
                    arguments: tool_call.arguments.clone(),
                });

                let tool_response = tokio::select! {
                    biased;
                    () = cancel.token().cancelled() => {
                        self.log_tool_event(
                            "tool_call_failed",
                            json!({
                                "iteration": iteration,
                                "server": server,
                                "tool": tool,
                                "ok": false,
                                "error": "cancelled",
                            }),
                        )
                        .await;
                        events.emit(AgentEvent::ToolFinish {
                            id: tool_call_id,
                            ok: false,
                            output: json!({ "error": "cancelled" }),
                        });
                        set_cancel_or_duration_stop(
                            &cancel,
                            &metrics,
                            &limits,
                            &mut final_result,
                            &mut stop_reason,
                        );
                        break;
                    }
                    response = self.tools.call_tool(
                        qualified.server(),
                        qualified.tool(),
                        tool_call.arguments,
                    ) => response,
                };
                let tool_response = match tool_response {
                    Ok(value) => value,
                    Err(err) => {
                        warn!(
                            iteration,
                            tool = %qualified_tool,
                            error = %err,
                            "tool call failed; returning error to the model"
                        );
                        self.log_tool_event(
                            "tool_call_failed",
                            json!({
                                "iteration": iteration,
                                "server": server,
                                "tool": tool,
                                "ok": false,
                                "error": err.to_string(),
                            }),
                        )
                        .await;
                        events.emit(AgentEvent::ToolFinish {
                            id: tool_call_id,
                            ok: false,
                            output: json!({ "error": err.to_string() }),
                        });
                        push_message(
                            &mut messages,
                            &mut full_history,
                            ChatMessage::new(
                                ChatRole::User,
                                json!({
                                    "tool_result": {
                                        "server": server,
                                        "tool": tool,
                                        "error": err.to_string()
                                    }
                                })
                                .to_string(),
                            ),
                            &history,
                        );
                        prune_message_history(&mut messages, &history);
                        continue;
                    }
                };

                diag.tools_executed = diag.tools_executed.saturating_add(1);
                if server == "home" && tool == "write" {
                    diag.home_write_ok = true;
                }
                if server == "home" && tool == "run" {
                    diag.home_run_ok = true;
                }
                self.log_tool_event(
                    "tool_call_finished",
                    json!({
                        "iteration": iteration,
                        "server": server,
                        "tool": tool,
                        "ok": true,
                    }),
                )
                .await;

                events.emit(AgentEvent::ToolFinish {
                    id: tool_call_id,
                    ok: true,
                    output: tool_response.clone(),
                });

                if should_stop_for_duration(&metrics, &limits, &mut final_result, &mut stop_reason)
                {
                    break;
                }
                if should_stop_for_cancel(
                    &cancel,
                    &metrics,
                    &limits,
                    &mut final_result,
                    &mut stop_reason,
                ) {
                    break;
                }
                push_message(
                    &mut messages,
                    &mut full_history,
                    ChatMessage::new(
                        ChatRole::User,
                        json!({
                            "tool_result": {
                                "server": server,
                                "tool": tool,
                                "result": tool_response
                            }
                        })
                        .to_string(),
                    ),
                    &history,
                );
                prune_message_history(&mut messages, &history);
            }

            if stop_reason == StopReason::LimitReached && !final_result.is_empty() {
                break;
            }

            if directive.done {
                final_result = directive
                    .result
                    .unwrap_or("Agent marked done without explicit result".to_string());
                stop_reason = StopReason::GoalReached;
                if !diag.home_run_ok {
                    warn!(
                        iterations = iteration,
                        parse_failures = diag.parse_failures,
                        tools_executed = diag.tools_executed,
                        home_write_ok = diag.home_write_ok,
                        "goal_reached without successful home.run"
                    );
                }
                info!(iterations = iteration, "agent goal reached");
                break;
            }

            prune_message_history(&mut messages, &history);
        }

        if !final_result.is_empty()
            && !cancel.is_cancelled()
            && self
                .pipeline_events
                .has_handlers(EventStage::RunBeforeOutput)
        {
            let transformed = self
                .pipeline_events
                .dispatch(
                    EventStage::RunBeforeOutput,
                    json!({ "result": final_result }),
                    None,
                    cancel.token(),
                )
                .await;
            let transformed = match transformed {
                Ok(payload) => payload,
                Err(err) => {
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err((
                        err.into(),
                        build_usage_report(&metrics, cost_tracker.report()),
                    ));
                }
            };
            final_result = match decode_run_before_output(transformed) {
                Ok(result) => result,
                Err(err) => {
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err((
                        err.into(),
                        build_usage_report(&metrics, cost_tracker.report()),
                    ));
                }
            };
        }

        let final_cost_report = cost_tracker.report();
        self.emit_run_summary(&diag, &stop_reason, &final_result, &final_cost_report)
            .await;

        if !final_result.is_empty() {
            events.emit(AgentEvent::Message(final_result.clone()));
        }

        let tokens = metrics.tokens();
        let output = RunOutput {
            run_id: self.run_id.clone(),
            result: final_result,
            usage: UsageReport {
                iterations: metrics.iterations(),
                prompt_tokens: tokens.prompt_tokens,
                completion_tokens: tokens.completion_tokens,
                total_tokens: tokens.total_tokens,
                elapsed_ms: metrics.elapsed_ms(),
                cost: final_cost_report,
            },
            stop_reason,
            logs: self.loggers.report(),
        };
        events.emit(AgentEvent::UsageSummary(output.usage.clone()));
        self.loggers
            .persist_chat_history(&full_history, Some(&output))
            .await;
        Ok(output)
    }

    async fn log_tool_event(&self, event_type: &str, payload: Value) {
        let sink = self.loggers.mcp.as_ref().or(self.loggers.ai.as_ref());
        let Some(sink) = sink else {
            return;
        };
        if let Err(err) = sink.write_event(event_type, payload).await {
            warn!(error = ?err, event_type, "failed to write tool lifecycle log event");
        }
    }

    async fn emit_run_summary(
        &self,
        diag: &RunDiagnostics,
        stop_reason: &StopReason,
        final_result: &str,
        cost: &RunCostReport,
    ) {
        let done_without_home_run = *stop_reason == StopReason::GoalReached && !diag.home_run_ok;
        let stop_reason_name = stop_reason_name(stop_reason);
        let audit_write_failed = self.loggers.audit_write_failed();
        info!(
            stop_reason = stop_reason_name,
            parse_failures = diag.parse_failures,
            tools_executed = diag.tools_executed,
            home_write_ok = diag.home_write_ok,
            home_run_ok = diag.home_run_ok,
            done_without_home_run,
            audit_write_failed,
            "agent run finished"
        );
        let Some(ai_log) = &self.loggers.ai else {
            return;
        };
        if let Err(err) = ai_log
            .write_event(
                "run_summary",
                json!({
                    "run_id": self.run_id,
                    "stop_reason": stop_reason_name,
                    "result_len": final_result.len(),
                    "parse_failures": diag.parse_failures,
                    "tools_executed": diag.tools_executed,
                    "home_write_ok": diag.home_write_ok,
                    "home_run_ok": diag.home_run_ok,
                    "done_without_home_run": done_without_home_run,
                    "audit_write_failed": audit_write_failed,
                    "cost": cost,
                }),
            )
            .await
        {
            warn!(error = ?err, "failed to write run_summary log event");
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct RunDiagnostics {
    parse_failures: u32,
    tools_executed: u32,
    home_write_ok: bool,
    home_run_ok: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextBeforeModelPayload {
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelAfterResponsePayload {
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunBeforeOutputPayload {
    result: String,
}

fn decode_context_before_model(
    canonical: &[ChatMessage],
    payload: Value,
) -> Result<Vec<ChatMessage>, EventMcpError> {
    let transformed: ContextBeforeModelPayload =
        serde_json::from_value(payload).map_err(|error| EventMcpError::InvalidPayload {
            event: EventStage::ContextBeforeModel,
            reason: error.to_string(),
        })?;
    let Some(original_system) = canonical.first() else {
        return Err(EventMcpError::InvalidPayload {
            event: EventStage::ContextBeforeModel,
            reason: "canonical message list is empty".to_string(),
        });
    };
    let Some(transformed_system) = transformed.messages.first() else {
        return Err(EventMcpError::InvalidPayload {
            event: EventStage::ContextBeforeModel,
            reason: "messages must not be empty".to_string(),
        });
    };
    if !matches!(original_system.role, ChatRole::System)
        || !matches!(transformed_system.role, ChatRole::System)
        || transformed_system.content != original_system.content
    {
        return Err(EventMcpError::InvalidPayload {
            event: EventStage::ContextBeforeModel,
            reason: "the original system message must remain the first message unchanged"
                .to_string(),
        });
    }
    Ok(transformed.messages)
}

fn decode_model_after_response(payload: Value) -> Result<String, EventMcpError> {
    serde_json::from_value::<ModelAfterResponsePayload>(payload)
        .map(|value| value.content)
        .map_err(|error| EventMcpError::InvalidPayload {
            event: EventStage::ModelAfterResponse,
            reason: error.to_string(),
        })
}

fn decode_run_before_output(payload: Value) -> Result<String, EventMcpError> {
    serde_json::from_value::<RunBeforeOutputPayload>(payload)
        .map(|value| value.result)
        .map_err(|error| EventMcpError::InvalidPayload {
            event: EventStage::RunBeforeOutput,
            reason: error.to_string(),
        })
}

fn build_user_message(
    prompt: &str,
    input_files_context: &str,
    available_tools: &[String],
) -> String {
    let attached_files = if input_files_context.is_empty() {
        "Attached input files: none".to_string()
    } else {
        format!("Attached input files (read-only context):\n{input_files_context}")
    };
    format!(
        "Goal: {prompt}\n\n{attached_files}\n\nAvailable tools: {tools}\n\nRespond with JSON only. No markdown fences. No text outside JSON.\n\nRequired response shape:\n{{\"done\": bool, \"thought\": string, \"tool_calls\": [{{\"server\":\"home\", \"tool\":\"write\", \"arguments\": {{\"path\":\"out/file.md\", \"content\":\"...\"}}}}], \"result\": string|null}}\n\nRules:\n- Use done=false and tool_calls while files still need to be written.\n- Use server=\"home\" for filesystem tools.\n- Set done=true only after required files were written.\n- Never describe tool calls in plain text.",
        tools = available_tools.join(", ")
    )
}

fn limit_name(limit: LimitExceeded) -> &'static str {
    match limit {
        LimitExceeded::Iterations => "max_iterations",
        LimitExceeded::Tokens => "max_tokens",
        LimitExceeded::Duration => "max_duration_sec",
    }
}

fn build_usage_report(metrics: &RunMetrics, cost: RunCostReport) -> UsageReport {
    let tokens = metrics.tokens();
    UsageReport {
        iterations: metrics.iterations(),
        prompt_tokens: tokens.prompt_tokens,
        completion_tokens: tokens.completion_tokens,
        total_tokens: tokens.total_tokens,
        elapsed_ms: metrics.elapsed_ms(),
        cost,
    }
}

fn generate_run_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("run-{timestamp:032x}-{:016x}", rand::random::<u64>())
}

fn stop_reason_name(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::GoalReached => "goal_reached",
        StopReason::LimitReached => "limit_reached",
        StopReason::Error => "error",
    }
}

fn should_stop_for_duration(
    metrics: &RunMetrics,
    limits: &LimitsConfig,
    final_result: &mut String,
    stop_reason: &mut StopReason,
) -> bool {
    if metrics.duration_limit_hit(limits) {
        *final_result = "Execution stopped due to limit: max_duration_sec".to_string();
        *stop_reason = StopReason::LimitReached;
        return true;
    }
    false
}

fn should_stop_for_cancel(
    cancel: &RunCancel,
    metrics: &RunMetrics,
    limits: &LimitsConfig,
    final_result: &mut String,
    stop_reason: &mut StopReason,
) -> bool {
    if cancel.is_cancelled() {
        set_cancel_or_duration_stop(cancel, metrics, limits, final_result, stop_reason);
        return true;
    }
    false
}

fn set_cancel_or_duration_stop(
    _cancel: &RunCancel,
    metrics: &RunMetrics,
    limits: &LimitsConfig,
    final_result: &mut String,
    stop_reason: &mut StopReason,
) {
    if metrics.duration_limit_hit(limits) {
        *final_result = "Execution stopped due to limit: max_duration_sec".to_string();
    } else {
        *final_result = "Execution cancelled by user".to_string();
    }
    *stop_reason = StopReason::LimitReached;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tokio::sync::mpsc;

    use super::*;
    use crate::limits::TokenUsage;
    use crate::logging::sink::MemoryEventSink;
    use crate::provider::ModelResponse;

    struct FailingProvider {
        calls: AtomicUsize,
        usage: TokenUsage,
    }

    #[async_trait]
    impl ModelClient for FailingProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
        ) -> Result<ModelResponse, crate::provider::Error> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(ModelResponse {
                    content: r#"{"done":false,"thought":"x","tool_calls":[],"result":null}"#
                        .to_string(),
                    usage: self.usage,
                })
            } else {
                Err(crate::provider::Error::EmptyChoices)
            }
        }
    }

    struct ScriptedProvider {
        responses: Vec<String>,
        calls: AtomicUsize,
        usage: TokenUsage,
    }

    #[async_trait]
    impl ModelClient for ScriptedProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
        ) -> Result<ModelResponse, crate::provider::Error> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let content = self
                .responses
                .get(call)
                .cloned()
                .ok_or(crate::provider::Error::EmptyChoices)?;
            Ok(ModelResponse {
                content,
                usage: self.usage,
            })
        }
    }

    struct RecordingProvider {
        responses: Vec<String>,
        calls: AtomicUsize,
        seen_messages: std::sync::Mutex<Vec<Vec<ChatMessage>>>,
    }

    #[async_trait]
    impl ModelClient for RecordingProvider {
        async fn complete(
            &self,
            messages: &[ChatMessage],
        ) -> Result<ModelResponse, crate::provider::Error> {
            self.seen_messages
                .lock()
                .expect("seen messages")
                .push(messages.to_vec());
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let content = self
                .responses
                .get(call)
                .cloned()
                .ok_or(crate::provider::Error::EmptyChoices)?;
            Ok(ModelResponse {
                content,
                usage: test_usage(),
            })
        }
    }

    #[derive(Default)]
    struct TransformingPipeline {
        context_inputs: std::sync::Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl PipelineEvents for TransformingPipeline {
        fn has_handlers(&self, _event: EventStage) -> bool {
            true
        }

        async fn dispatch(
            &self,
            event: EventStage,
            mut payload: Value,
            iteration: Option<u32>,
            _cancel: &tokio_util::sync::CancellationToken,
        ) -> Result<Value, EventMcpError> {
            match event {
                EventStage::ContextBeforeModel => {
                    self.context_inputs
                        .lock()
                        .expect("context inputs")
                        .push(payload.clone());
                    payload["messages"][1]["content"] = json!("COMPRESSED");
                    Ok(payload)
                }
                EventStage::ModelAfterResponse => {
                    let done = iteration == Some(2);
                    Ok(json!({
                        "content": json!({
                            "done": done,
                            "thought": "repaired",
                            "tool_calls": [],
                            "result": done.then_some("answer")
                        }).to_string()
                    }))
                }
                EventStage::RunBeforeOutput => Ok(json!({
                    "result": format!("formatted: {}", payload["result"].as_str().unwrap_or(""))
                })),
            }
        }
    }

    struct NoopTools;

    #[async_trait]
    impl ToolExecutor for NoopTools {
        async fn call_tool(
            &self,
            _server: &str,
            _tool: &str,
            _arguments: Value,
        ) -> Result<Value, crate::tool_api::ToolError> {
            Ok(Value::Null)
        }

        fn available_tools(&self) -> Vec<String> {
            Vec::new()
        }
    }

    struct RecordingTools {
        calls: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl ToolExecutor for RecordingTools {
        async fn call_tool(
            &self,
            server: &str,
            tool: &str,
            _arguments: Value,
        ) -> Result<Value, crate::tool_api::ToolError> {
            self.calls
                .lock()
                .unwrap()
                .push((server.to_string(), tool.to_string()));
            Ok(json!({"ok": true, "stdout": "11"}))
        }

        fn available_tools(&self) -> Vec<String> {
            vec!["home.write".to_string(), "home.run".to_string()]
        }
    }

    fn test_usage() -> TokenUsage {
        TokenUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        }
    }

    fn test_limits() -> LimitsConfig {
        LimitsConfig {
            max_iterations: 10,
            max_tokens: 1_000,
            max_duration_sec: 100,
            max_cost: None,
        }
    }

    fn test_history() -> ProviderHistoryConfig {
        ProviderHistoryConfig::default()
    }

    #[test]
    fn user_message_contains_read_only_file_context() {
        let message = build_user_message(
            "review",
            "--- file: input.rs ---\ncode\n--- end file ---",
            &["home.read".to_string()],
        );

        assert!(message.contains("Goal: review"));
        assert!(message.contains("read-only context"));
        assert!(message.contains("input.rs"));
        assert!(message.contains("home.read"));
    }

    #[tokio::test]
    async fn pipeline_transforms_provider_snapshot_response_and_final_output() {
        let provider = Arc::new(RecordingProvider {
            responses: vec!["invalid first".to_string(), "invalid second".to_string()],
            calls: AtomicUsize::new(0),
            seen_messages: std::sync::Mutex::new(Vec::new()),
        });
        let pipeline = Arc::new(TransformingPipeline::default());
        let engine = AgentEngine::new(provider.clone(), Arc::new(NoopTools), Loggers::default())
            .with_pipeline_events(pipeline.clone());

        let output = engine
            .run(AgentRunRequest {
                prompt: "test".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: test_limits(),
                history: test_history(),
                cancel: RunCancel::new(),
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::GoalReached);
        assert_eq!(output.result, "formatted: answer");

        let seen = provider.seen_messages.lock().expect("seen messages");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0][1].content.as_ref(), "COMPRESSED");
        assert_eq!(seen[1][1].content.as_ref(), "COMPRESSED");

        let context_inputs = pipeline.context_inputs.lock().expect("context inputs");
        assert_eq!(context_inputs.len(), 2);
        let second_canonical_user = context_inputs[1]["messages"][1]["content"]
            .as_str()
            .expect("canonical user content");
        assert!(second_canonical_user.contains("Goal: test"));
        assert_ne!(second_canonical_user, "COMPRESSED");
    }

    #[tokio::test]
    async fn error_path_preserves_partial_usage() {
        let usage = TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        };
        let provider = Arc::new(FailingProvider {
            calls: AtomicUsize::new(0),
            usage,
        });
        let tools = Arc::new(NoopTools);
        let engine = AgentEngine::new(provider, tools, Loggers::default());
        let output = engine
            .run(AgentRunRequest {
                prompt: "test".to_string(),
                system_prompt: "system prompt".to_string(),
                input_files_context: String::new(),
                limits: test_limits(),
                history: test_history(),
                cancel: RunCancel::new(),
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::Error);
        assert_eq!(output.usage.iterations, 2);
        assert_eq!(output.usage.prompt_tokens, 10);
        assert_eq!(output.usage.completion_tokens, 5);
        assert_eq!(output.usage.total_tokens, 15);
    }

    #[tokio::test]
    async fn multi_json_parse_failure_logs_and_done_without_tools() {
        let multi = concat!(
            r#"{"done":false,"thought":"fetch","tool_calls":[{"server":"aoc","tool":"aoc_get_task","arguments":{}}],"result":null}"#,
            "\n\n",
            r#"{"done":false,"thought":"write","tool_calls":[{"server":"home","tool":"write","arguments":{"path":"solution.py","content":"print(1)"}}],"result":null}"#,
        );
        let provider = Arc::new(ScriptedProvider {
            responses: vec![
                multi.to_string(),
                r#"{"done":true,"thought":"guess","tool_calls":[],"result":"2164381"}"#.to_string(),
            ],
            calls: AtomicUsize::new(0),
            usage: test_usage(),
        });
        let tools = Arc::new(RecordingTools {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let ai_sink = Arc::new(MemoryEventSink::new());
        let mcp_sink = Arc::new(MemoryEventSink::new());
        let loggers = Loggers::with_sinks(Some(ai_sink.clone()), Some(mcp_sink.clone()));
        let engine = AgentEngine::new(provider, tools.clone(), loggers);
        let output = engine
            .run(AgentRunRequest {
                prompt: "solve".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: test_limits(),
                history: test_history(),
                cancel: RunCancel::new(),
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::GoalReached);
        assert_eq!(output.result, "2164381");
        assert!(tools.calls.lock().unwrap().is_empty());

        let ai_events = ai_sink.events();
        assert!(
            ai_events
                .iter()
                .any(|(name, _)| name == "directive_parse_failed"),
            "expected directive_parse_failed in ai log: {ai_events:?}"
        );
        let summary = ai_events
            .iter()
            .find(|(name, _)| name == "run_summary")
            .map(|(_, payload)| payload.clone())
            .expect("run_summary");
        assert_eq!(summary["parse_failures"], 1);
        assert_eq!(summary["tools_executed"], 0);
        assert_eq!(summary["home_run_ok"], false);
        assert_eq!(summary["done_without_home_run"], true);
        assert!(mcp_sink.events().is_empty());
    }

    #[tokio::test]
    async fn successful_home_run_is_recorded_in_summary() {
        let provider = Arc::new(ScriptedProvider {
            responses: vec![
                r#"{"done":false,"thought":"run","tool_calls":[{"server":"home","tool":"run","arguments":{"program":"python","args":["solution.py"]}}],"result":null}"#
                    .to_string(),
                r#"{"done":true,"thought":"ok","tool_calls":[],"result":"11"}"#.to_string(),
            ],
            calls: AtomicUsize::new(0),
            usage: test_usage(),
        });
        let tools = Arc::new(RecordingTools {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let ai_sink = Arc::new(MemoryEventSink::new());
        let mcp_sink = Arc::new(MemoryEventSink::new());
        let loggers = Loggers::with_sinks(Some(ai_sink.clone()), Some(mcp_sink.clone()));
        let engine = AgentEngine::new(provider, tools.clone(), loggers);
        let output = engine
            .run(AgentRunRequest {
                prompt: "solve".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: test_limits(),
                history: test_history(),
                cancel: RunCancel::new(),
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::GoalReached);
        assert_eq!(output.result, "11");
        assert_eq!(
            tools.calls.lock().unwrap().as_slice(),
            &[("home".to_string(), "run".to_string())]
        );

        let mcp_events = mcp_sink.events();
        assert!(mcp_events
            .iter()
            .any(|(name, _)| name == "tool_call_started"));
        assert!(mcp_events
            .iter()
            .any(|(name, _)| name == "tool_call_finished"));

        let summary = ai_sink
            .events()
            .into_iter()
            .find(|(name, _)| name == "run_summary")
            .map(|(_, payload)| payload)
            .expect("run_summary");
        assert_eq!(summary["tools_executed"], 1);
        assert_eq!(summary["home_run_ok"], true);
        assert_eq!(summary["done_without_home_run"], false);
        assert_eq!(summary["parse_failures"], 0);
        assert_eq!(summary["audit_write_failed"], false);
    }

    #[tokio::test]
    async fn audit_sink_failure_does_not_abort_run() {
        use crate::logging::sink::FailingEventSink;

        let provider = Arc::new(ScriptedProvider {
            responses: vec![
                r#"{"done":false,"thought":"run","tool_calls":[{"server":"home","tool":"run","arguments":{"program":"python","args":["solution.py"]}}],"result":null}"#
                    .to_string(),
                r#"{"done":true,"thought":"ok","tool_calls":[],"result":"11"}"#.to_string(),
            ],
            calls: AtomicUsize::new(0),
            usage: test_usage(),
        });
        let tools = Arc::new(RecordingTools {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let failing: crate::logging::SharedEventSink = Arc::new(FailingEventSink::new());
        let loggers = Loggers::with_sinks(Some(failing.clone()), Some(failing));
        assert!(!loggers.audit_write_failed());
        let engine = AgentEngine::new(provider, tools.clone(), loggers.clone());
        let output = engine
            .run(AgentRunRequest {
                prompt: "solve".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: test_limits(),
                history: test_history(),
                cancel: RunCancel::new(),
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::GoalReached);
        assert_eq!(output.result, "11");
        assert_eq!(tools.calls.lock().unwrap().len(), 1);
        assert!(
            loggers.audit_write_failed(),
            "failing audit sink must set audit_write_failed"
        );
    }

    struct SlowProvider;

    #[async_trait]
    impl ModelClient for SlowProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
        ) -> Result<ModelResponse, crate::provider::Error> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(ModelResponse {
                content: r#"{"done":true,"thought":"late","tool_calls":[],"result":"too late"}"#
                    .to_string(),
                usage: test_usage(),
            })
        }
    }

    #[tokio::test]
    async fn short_deadline_cancels_slow_provider_without_waiting() {
        let provider = Arc::new(SlowProvider);
        let tools = Arc::new(NoopTools);
        let engine = AgentEngine::new(provider, tools, Loggers::default());
        let started = std::time::Instant::now();
        let output = engine
            .run(AgentRunRequest {
                prompt: "test".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: LimitsConfig {
                    max_iterations: 10,
                    max_tokens: 1_000,
                    max_duration_sec: 1,
                    max_cost: None,
                },
                history: test_history(),
                cancel: RunCancel::new(),
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::LimitReached);
        assert!(
            output.result.contains("max_duration_sec"),
            "result was: {}",
            output.result
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "run should exit on deadline, not wait for slow provider (elapsed {:?})",
            started.elapsed()
        );
    }

    struct SlowTools;

    #[async_trait]
    impl ToolExecutor for SlowTools {
        async fn call_tool(
            &self,
            _server: &str,
            _tool: &str,
            _arguments: Value,
        ) -> Result<Value, crate::tool_api::ToolError> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(json!({"ok": true}))
        }

        fn available_tools(&self) -> Vec<String> {
            vec!["home.run".to_string()]
        }
    }

    #[tokio::test]
    async fn explicit_cancel_returns_cancelled_message() {
        let provider = Arc::new(SlowProvider);
        let tools = Arc::new(NoopTools);
        let engine = AgentEngine::new(provider, tools, Loggers::default());
        let cancel = RunCancel::new();
        cancel.cancel();

        let output = engine
            .run(AgentRunRequest {
                prompt: "test".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: test_limits(),
                history: test_history(),
                cancel,
                events: crate::agent::AgentEventTx::noop(),
            })
            .await;

        assert_eq!(output.stop_reason, StopReason::LimitReached);
        assert_eq!(output.result, "Execution cancelled by user");
    }

    #[tokio::test]
    async fn cancel_during_tool_call_emits_tool_finish() {
        let provider = Arc::new(ScriptedProvider {
            responses: vec![r#"{"done":false,"thought":"run","tool_calls":[{"server":"home","tool":"run","arguments":{"program":"python","args":["solution.py"]}}],"result":null}"#
                .to_string()],
            calls: AtomicUsize::new(0),
            usage: test_usage(),
        });
        let tools = Arc::new(SlowTools);
        let engine = AgentEngine::new(provider, tools, Loggers::default());
        let cancel = RunCancel::new();
        let cancel_signal = cancel.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_signal.cancel();
        });

        let output = engine
            .run(AgentRunRequest {
                prompt: "test".to_string(),
                system_prompt: "system".to_string(),
                input_files_context: String::new(),
                limits: LimitsConfig {
                    max_iterations: 10,
                    max_tokens: 1_000,
                    max_duration_sec: 60,
                    max_cost: None,
                },
                history: test_history(),
                cancel,
                events: crate::agent::AgentEventTx::from_sender(tx),
            })
            .await;

        let mut saw_tool_start = false;
        let mut saw_cancelled_tool_finish = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentEvent::ToolStart { .. } => saw_tool_start = true,
                AgentEvent::ToolFinish { ok, output, .. } => {
                    if !ok && output["error"] == "cancelled" {
                        saw_cancelled_tool_finish = true;
                    }
                }
                AgentEvent::UsageSummary(_) => {}
                AgentEvent::Thought(_) | AgentEvent::Message(_) => {}
            }
        }

        assert_eq!(output.stop_reason, StopReason::LimitReached);
        assert_eq!(output.result, "Execution cancelled by user");
        assert!(saw_tool_start);
        assert!(saw_cancelled_tool_finish);
    }
}
