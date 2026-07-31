use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{info, instrument, warn};

use crate::access::QualifiedTool;
use crate::config::ProviderHistoryConfig;
use crate::limits::{LimitExceeded, LimitsConfig, RunMetrics};
use crate::logging::{Loggers, LoggingError};
use crate::mcp::ToolExecutor;
use crate::output::{RunOutput, StopReason, UsageReport};
use crate::provider::{ChatMessage, ChatRole, ModelClient};

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider failure: {0}")]
    Provider(#[from] crate::provider::Error),
    #[error("tool failure: {0}")]
    Tool(#[from] crate::tools::ToolError),
    #[error("failed to decode model directive: {0}")]
    DirectiveDecode(#[from] serde_json::Error),
    #[error("internal logging failure: {0}")]
    Logging(#[from] LoggingError),
}

/// Prefix length: system prompt + initial user goal (not configurable).
const HISTORY_PREFIX_LEN: usize = 2;

#[derive(Clone)]
pub struct AgentRunRequest {
    pub prompt: String,
    pub system_prompt: String,
    pub input_files_context: String,
    pub limits: LimitsConfig,
    /// Model context-window pruning budgets (`provider.history`).
    pub history: ProviderHistoryConfig,
}

pub struct AgentEngine {
    model: Arc<dyn ModelClient>,
    tools: Arc<dyn ToolExecutor>,
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
            loggers,
        }
    }

    pub async fn run(&self, request: AgentRunRequest) -> RunOutput {
        let result = self.run_inner(request).await;
        self.loggers.shutdown().await;
        match result {
            Ok(out) => out,
            Err((err, usage)) => RunOutput {
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
        } = request;

        let available_tools = self.tools.available_tools();
        let user_message = build_user_message(&prompt, &input_files_context, &available_tools);
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, system_prompt),
            ChatMessage::new(ChatRole::User, user_message),
        ];
        let mut full_history = messages.clone();
        let mut metrics = RunMetrics::new();
        let mut final_result = String::new();
        let mut stop_reason = StopReason::LimitReached;
        let mut diag = RunDiagnostics::default();

        loop {
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

            let completion = match self.model.complete(&messages).await {
                Ok(completion) => completion,
                Err(err) => {
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err((err.into(), build_usage_report(&metrics)));
                }
            };
            metrics.add_tokens(completion.usage);
            if metrics.tokens_limit_hit(&limits) {
                final_result = "Execution stopped due to limit: max_tokens".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }
            if metrics.duration_limit_hit(&limits) {
                final_result = "Execution stopped due to limit: max_duration_sec".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }

            if let Some(ai_log) = &self.loggers.ai {
                if let Err(err) = ai_log
                    .write_event(
                        "ai_completion",
                        json!({
                            "iteration": iteration,
                            "content": completion.content,
                            "usage": completion.usage,
                        }),
                    )
                    .await
                {
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err((err.into(), build_usage_report(&metrics)));
                }
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
                            self.loggers.persist_chat_history(&full_history, None).await;
                            return Err((log_err.into(), build_usage_report(&metrics)));
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

            for tool_call in directive.tool_calls {
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

                let tool_response = match self
                    .tools
                    .call_tool(qualified.server(), qualified.tool(), tool_call.arguments)
                    .await
                {
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

                if metrics.duration_limit_hit(&limits) {
                    final_result = "Execution stopped due to limit: max_duration_sec".to_string();
                    stop_reason = StopReason::LimitReached;
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

        self.emit_run_summary(&diag, &stop_reason, &final_result)
            .await;

        let tokens = metrics.tokens();
        let output = RunOutput {
            result: final_result,
            usage: UsageReport {
                iterations: metrics.iterations(),
                prompt_tokens: tokens.prompt_tokens,
                completion_tokens: tokens.completion_tokens,
                total_tokens: tokens.total_tokens,
                elapsed_ms: metrics.elapsed_ms(),
            },
            stop_reason,
            logs: self.loggers.report(),
        };
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
            warn!(error = %err, event_type, "failed to write tool lifecycle log event");
        }
    }

    async fn emit_run_summary(
        &self,
        diag: &RunDiagnostics,
        stop_reason: &StopReason,
        final_result: &str,
    ) {
        let done_without_home_run = *stop_reason == StopReason::GoalReached && !diag.home_run_ok;
        let stop_reason_name = stop_reason_name(stop_reason);
        info!(
            stop_reason = stop_reason_name,
            parse_failures = diag.parse_failures,
            tools_executed = diag.tools_executed,
            home_write_ok = diag.home_write_ok,
            home_run_ok = diag.home_run_ok,
            done_without_home_run,
            "agent run finished"
        );
        let Some(ai_log) = &self.loggers.ai else {
            return;
        };
        if let Err(err) = ai_log
            .write_event(
                "run_summary",
                json!({
                    "stop_reason": stop_reason_name,
                    "result_len": final_result.len(),
                    "parse_failures": diag.parse_failures,
                    "tools_executed": diag.tools_executed,
                    "home_write_ok": diag.home_write_ok,
                    "home_run_ok": diag.home_run_ok,
                    "done_without_home_run": done_without_home_run,
                }),
            )
            .await
        {
            warn!(error = %err, "failed to write run_summary log event");
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

fn push_message(
    messages: &mut Vec<ChatMessage>,
    full_history: &mut Vec<ChatMessage>,
    message: ChatMessage,
    history: &ProviderHistoryConfig,
) {
    full_history.push(message.clone());
    prune_message_history(full_history, history);
    messages.push(message);
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

fn build_usage_report(metrics: &RunMetrics) -> UsageReport {
    let tokens = metrics.tokens();
    UsageReport {
        iterations: metrics.iterations(),
        prompt_tokens: tokens.prompt_tokens,
        completion_tokens: tokens.completion_tokens,
        total_tokens: tokens.total_tokens,
        elapsed_ms: metrics.elapsed_ms(),
    }
}

/// Keeps the system prompt and initial user message, dropping oldest middle turns
/// by message count and by total character budget from `history`.
fn prune_message_history(messages: &mut Vec<ChatMessage>, history: &ProviderHistoryConfig) {
    prune_by_message_count(messages, history);
    prune_by_char_budget(messages, history);
}

fn prune_by_message_count(messages: &mut Vec<ChatMessage>, history: &ProviderHistoryConfig) {
    let max_tail = history.max_tail_messages;
    let max_total = HISTORY_PREFIX_LEN.saturating_add(max_tail);
    if messages.len() <= max_total {
        return;
    }
    let tail_start = messages.len() - max_tail;
    if tail_start <= HISTORY_PREFIX_LEN {
        return;
    }
    let tail: Vec<ChatMessage> = messages.drain(tail_start..).collect();
    messages.truncate(HISTORY_PREFIX_LEN);
    messages.extend(tail);
}

fn prune_by_char_budget(messages: &mut Vec<ChatMessage>, history: &ProviderHistoryConfig) {
    while messages.len() > HISTORY_PREFIX_LEN && history_char_len(messages) > history.max_chars {
        // Drop the oldest non-prefix turn; prefer retaining recent context.
        messages.remove(HISTORY_PREFIX_LEN);
    }
}

fn history_char_len(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|message| message.content.chars().count())
        .sum()
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
struct ToolCallDirective {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelDirective {
    done: bool,
    #[allow(dead_code)]
    thought: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDirective>,
    #[serde(default)]
    result: Option<String>,
}

fn parse_directive(raw: &str) -> Result<ModelDirective, serde_json::Error> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed);
    }

    // Some models still wrap JSON in Markdown fences.
    let stripped = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    serde_json::from_str(stripped)
}

fn stop_reason_name(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::GoalReached => "goal_reached",
        StopReason::LimitReached => "limit_reached",
        StopReason::Error => "error",
    }
}

/// Rough count of top-level JSON objects in a model reply (detects multi-JSON turns).
fn approx_json_object_count(content: &str) -> usize {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let mut count = 0usize;
    if trimmed.starts_with('{') {
        count = count.saturating_add(1);
    }
    count = count.saturating_add(trimmed.matches("\n{").count());
    count
}

fn content_preview(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    let mut preview: String = trimmed.chars().take(max_chars).collect();
    if trimmed.chars().count() > max_chars {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::Value;

    use super::*;
    use crate::limits::TokenUsage;
    use crate::logging::MemoryEventSink;
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

    struct NoopTools;

    #[async_trait]
    impl ToolExecutor for NoopTools {
        async fn call_tool(
            &self,
            _server: &str,
            _tool: &str,
            _arguments: Value,
        ) -> Result<Value, crate::tools::ToolError> {
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
        ) -> Result<Value, crate::tools::ToolError> {
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

    #[test]
    fn parse_directive_accepts_plain_json() {
        let directive =
            parse_directive(r#"{"done":true,"thought":"ok","tool_calls":[],"result":"finished"}"#)
                .expect("plain json should parse");

        assert!(directive.done);
        assert_eq!(directive.result.as_deref(), Some("finished"));
        assert!(directive.tool_calls.is_empty());
    }

    #[test]
    fn parse_directive_strips_markdown_fences() {
        let directive = parse_directive(
            "```json\n{\"done\":false,\"thought\":\"step\",\"tool_calls\":[],\"result\":null}\n```",
        )
        .expect("fenced json should parse");

        assert!(!directive.done);
        assert_eq!(directive.thought.as_deref(), Some("step"));
    }

    #[test]
    fn parse_directive_rejects_invalid_json() {
        assert!(parse_directive("not json at all").is_err());
        assert!(parse_directive("```json\n{broken\n```").is_err());
    }

    #[test]
    fn approx_json_object_count_detects_multi_json() {
        let multi = concat!(
            r#"{"done":false,"thought":"a","tool_calls":[],"result":null}"#,
            "\n\n",
            r#"{"done":true,"thought":"b","tool_calls":[],"result":"1"}"#,
        );
        assert_eq!(approx_json_object_count(multi), 2);
        assert_eq!(
            approx_json_object_count(r#"{"done":true,"tool_calls":[],"result":null}"#),
            1
        );
        assert_eq!(approx_json_object_count(""), 0);
    }

    #[test]
    fn content_preview_truncates() {
        let preview = content_preview("abcdefghij", 4);
        assert_eq!(preview, "abcd…");
    }

    #[test]
    fn push_message_prunes_full_history_to_same_budget_as_messages() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        let mut full_history = messages.clone();

        for i in 0..40 {
            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::Assistant, format!("assistant-{i}")),
                &history,
            );
            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::User, format!("user-{i}")),
                &history,
            );
            prune_message_history(&mut messages, &history);
        }

        assert_eq!(full_history.len(), messages.len());
        assert!(full_history.len() <= HISTORY_PREFIX_LEN + history.max_tail_messages);
        assert_eq!(full_history[0].content.as_ref(), "system");
        assert_eq!(full_history[1].content.as_ref(), "goal");
        assert_eq!(full_history.last().unwrap().content.as_ref(), "user-39");
    }

    #[test]
    fn full_history_respects_char_budget() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        let mut full_history = messages.clone();

        push_message(
            &mut messages,
            &mut full_history,
            ChatMessage::new(ChatRole::Assistant, "a".repeat(80_000)),
            &history,
        );
        push_message(
            &mut messages,
            &mut full_history,
            ChatMessage::new(ChatRole::User, "b".repeat(80_000)),
            &history,
        );
        push_message(
            &mut messages,
            &mut full_history,
            ChatMessage::new(ChatRole::Assistant, "c".repeat(80_000)),
            &history,
        );

        assert!(history_char_len(&full_history) <= history.max_chars);
        assert_eq!(full_history[0].content.as_ref(), "system");
        assert_eq!(full_history[1].content.as_ref(), "goal");
        assert_eq!(
            full_history.last().unwrap().content.as_ref(),
            &"c".repeat(80_000)
        );
    }

    #[test]
    fn prune_message_history_keeps_prefix_and_tail() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        for i in 0..40 {
            messages.push(ChatMessage::new(
                ChatRole::Assistant,
                format!("assistant-{i}"),
            ));
            messages.push(ChatMessage::new(ChatRole::User, format!("user-{i}")));
        }

        prune_message_history(&mut messages, &history);

        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        assert!(messages.len() <= HISTORY_PREFIX_LEN + history.max_tail_messages);
        assert_eq!(messages.last().unwrap().content.as_ref(), "user-39");
    }

    #[test]
    fn prune_message_history_enforces_char_budget() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        messages.push(ChatMessage::new(ChatRole::Assistant, "a".repeat(80_000)));
        messages.push(ChatMessage::new(ChatRole::User, "b".repeat(80_000)));
        messages.push(ChatMessage::new(ChatRole::Assistant, "c".repeat(80_000)));

        prune_message_history(&mut messages, &history);

        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        assert!(history_char_len(&messages) <= history.max_chars);
        assert_eq!(messages.last().unwrap().content.chars().count(), 80_000);
        assert_eq!(
            messages.last().unwrap().content.as_ref(),
            &"c".repeat(80_000)
        );
        // Oldest oversized middle turn is dropped first.
        assert!(!messages
            .iter()
            .any(|message| message.content.as_ref() == "a".repeat(80_000)));
    }

    #[test]
    fn prune_message_history_handles_single_oversized_tool_result() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
            ChatMessage::new(ChatRole::User, "t".repeat(100_000)),
            ChatMessage::new(ChatRole::Assistant, "ok"),
        ];

        prune_message_history(&mut messages, &history);

        assert!(history_char_len(&messages) <= history.max_chars);
        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        // 100k fits under the 200k budget with prefix retained.
        assert!(messages
            .iter()
            .any(|message| message.content.chars().count() == 100_000));
    }

    #[test]
    fn prune_message_history_respects_larger_configured_window() {
        let history = ProviderHistoryConfig {
            max_tail_messages: 80,
            max_chars: 500_000,
        };
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        for i in 0..40 {
            messages.push(ChatMessage::new(
                ChatRole::Assistant,
                format!("assistant-{i}"),
            ));
            messages.push(ChatMessage::new(ChatRole::User, format!("user-{i}")));
        }

        let before_len = messages.len();
        prune_message_history(&mut messages, &history);

        assert_eq!(messages.len(), before_len);
        assert!(messages
            .iter()
            .any(|message| message.content.as_ref() == "assistant-0"));
        assert_eq!(messages.last().unwrap().content.as_ref(), "user-39");
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
    }
}
