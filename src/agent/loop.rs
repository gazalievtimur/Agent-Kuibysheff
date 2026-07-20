use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{info, instrument, warn};

use crate::limits::{LimitExceeded, LimitsConfig, RunMetrics};
use crate::logging::{LoggingError, Loggers};
use crate::mcp::ToolExecutor;
use crate::output::{RunOutput, StopReason, UsageReport};
use crate::provider::{ChatMessage, ChatRole, ModelClient};

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider failure: {0}")]
    Provider(#[from] crate::provider::Error),
    #[error("tool failure: {0}")]
    Tool(#[from] crate::mcp::Error),
    #[error("failed to decode model directive: {0}")]
    DirectiveDecode(#[from] serde_json::Error),
    #[error("internal logging failure: {0}")]
    Logging(#[from] LoggingError),
}

/// Maximum non-prefix messages retained after count-based pruning
/// (system + initial user are always kept).
const MAX_TAIL_MESSAGES: usize = 30;
/// Maximum total UTF-8 char budget for the working message window.
const MAX_HISTORY_CHARS: usize = 200_000;
/// Prefix length: system prompt + initial user goal.
const HISTORY_PREFIX_LEN: usize = 2;

#[derive(Clone)]
pub struct AgentRunRequest {
    pub prompt: String,
    pub system_prompt: String,
    pub input_files_context: String,
    pub limits: LimitsConfig,
    /// `None` allows all tools; `Some(set)` enforces the skills policy.
    pub allowed_tools: Option<HashSet<String>>,
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
        match self.run_inner(request).await {
            Ok(out) => out,
            Err(err) => RunOutput {
                result: err.to_string(),
                usage: UsageReport::default(),
                stop_reason: StopReason::Error,
                logs: self.loggers.report(),
            },
        }
    }

    #[instrument(skip(self, request), fields(prompt_len = request.prompt.len()))]
    #[allow(clippy::too_many_lines)]
    async fn run_inner(&self, request: AgentRunRequest) -> Result<RunOutput, AgentError> {
        let AgentRunRequest {
            prompt,
            system_prompt,
            input_files_context,
            limits,
            allowed_tools,
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

            let completion = match self.model.complete(&messages).await {
                Ok(completion) => completion,
                Err(err) => {
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err(err.into());
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
                            "iteration": metrics.iterations(),
                            "content": completion.content,
                            "usage": completion.usage,
                        }),
                    )
                    .await
                {
                    self.loggers.persist_chat_history(&full_history, None).await;
                    return Err(err.into());
                }
            }

            let directive = match parse_directive(&completion.content) {
                Ok(v) => v,
                Err(err) => {
                    push_message(
                        &mut messages,
                        &mut full_history,
                        ChatMessage::new(ChatRole::Assistant, completion.content),
                    );
                    push_message(
                        &mut messages,
                        &mut full_history,
                        ChatMessage::new(
                            ChatRole::User,
                            json!({
                                "parse_error": err.to_string(),
                                "hint": "Respond with strict JSON only. No markdown fences. Required shape: {\"done\": bool, \"thought\": string, \"tool_calls\": [...], \"result\": string|null}"
                            })
                            .to_string(),
                        ),
                    );
                    prune_message_history(&mut messages);
                    continue;
                }
            };

            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::Assistant, completion.content),
            );

            for tool_call in directive.tool_calls {
                let qualified_tool = format!("{}.{}", tool_call.server, tool_call.tool);
                if let Some(allowed) = &allowed_tools {
                    if !allowed.contains(&tool_call.tool) && !allowed.contains(&qualified_tool) {
                        warn!(
                            tool = %qualified_tool,
                            "tool call rejected by skills policy"
                        );
                        let warning = json!({
                            "tool_call": tool_call,
                            "error": "tool is not allowed by skills policy"
                        });
                        push_message(
                            &mut messages,
                            &mut full_history,
                            ChatMessage::new(ChatRole::User, warning.to_string()),
                        );
                        prune_message_history(&mut messages);
                        continue;
                    }
                }

                let tool_response = match self
                    .tools
                    .call_tool(&tool_call.server, &tool_call.tool, tool_call.arguments)
                    .await
                {
                    Ok(value) => value,
                    Err(err) => {
                        warn!(
                            tool = %qualified_tool,
                            error = %err,
                            "tool call failed; returning error to the model"
                        );
                        push_message(
                            &mut messages,
                            &mut full_history,
                            ChatMessage::new(
                                ChatRole::User,
                                json!({
                                    "tool_result": {
                                        "server": tool_call.server,
                                        "tool": tool_call.tool,
                                        "error": err.to_string()
                                    }
                                })
                                .to_string(),
                            ),
                        );
                        prune_message_history(&mut messages);
                        continue;
                    }
                };
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
                                "server": tool_call.server,
                                "tool": tool_call.tool,
                                "result": tool_response
                            }
                        })
                        .to_string(),
                    ),
                );
                prune_message_history(&mut messages);
            }

            if stop_reason == StopReason::LimitReached && !final_result.is_empty() {
                break;
            }

            if directive.done {
                final_result = directive
                    .result
                    .unwrap_or("Agent marked done without explicit result".to_string());
                stop_reason = StopReason::GoalReached;
                info!(iterations = metrics.iterations(), "agent goal reached");
                break;
            }

            prune_message_history(&mut messages);
        }

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
}

fn push_message(
    messages: &mut Vec<ChatMessage>,
    full_history: &mut Vec<ChatMessage>,
    message: ChatMessage,
) {
    full_history.push(message.clone());
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

/// Keeps the system prompt and initial user message, dropping oldest middle turns
/// by message count and by total character budget.
fn prune_message_history(messages: &mut Vec<ChatMessage>) {
    prune_by_message_count(messages);
    prune_by_char_budget(messages);
}

fn prune_by_message_count(messages: &mut Vec<ChatMessage>) {
    let max_total = HISTORY_PREFIX_LEN.saturating_add(MAX_TAIL_MESSAGES);
    if messages.len() <= max_total {
        return;
    }
    let tail_start = messages.len() - MAX_TAIL_MESSAGES;
    if tail_start <= HISTORY_PREFIX_LEN {
        return;
    }
    let tail: Vec<ChatMessage> = messages.drain(tail_start..).collect();
    messages.truncate(HISTORY_PREFIX_LEN);
    messages.extend(tail);
}

fn prune_by_char_budget(messages: &mut Vec<ChatMessage>) {
    while messages.len() > HISTORY_PREFIX_LEN && history_char_len(messages) > MAX_HISTORY_CHARS {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn push_message_keeps_full_history_when_messages_are_pruned() {
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
            );
            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::User, format!("user-{i}")),
            );
            prune_message_history(&mut messages);
        }

        assert!(full_history.len() > messages.len());
        assert_eq!(full_history.last().unwrap().content.as_ref(), "user-39");
    }

    #[test]
    fn prune_message_history_keeps_prefix_and_tail() {
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        for i in 0..40 {
            messages.push(ChatMessage::new(ChatRole::Assistant, format!("assistant-{i}")));
            messages.push(ChatMessage::new(ChatRole::User, format!("user-{i}")));
        }

        prune_message_history(&mut messages);

        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        assert!(messages.len() <= HISTORY_PREFIX_LEN + MAX_TAIL_MESSAGES);
        assert_eq!(messages.last().unwrap().content.as_ref(), "user-39");
    }

    #[test]
    fn prune_message_history_enforces_char_budget() {
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        messages.push(ChatMessage::new(ChatRole::Assistant, "a".repeat(80_000)));
        messages.push(ChatMessage::new(ChatRole::User, "b".repeat(80_000)));
        messages.push(ChatMessage::new(ChatRole::Assistant, "c".repeat(80_000)));

        prune_message_history(&mut messages);

        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        assert!(history_char_len(&messages) <= MAX_HISTORY_CHARS);
        assert_eq!(messages.last().unwrap().content.chars().count(), 80_000);
        assert_eq!(messages.last().unwrap().content.as_ref(), &"c".repeat(80_000));
        // Oldest oversized middle turn is dropped first.
        assert!(
            !messages
                .iter()
                .any(|message| message.content.as_ref() == "a".repeat(80_000))
        );
    }

    #[test]
    fn prune_message_history_handles_single_oversized_tool_result() {
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
            ChatMessage::new(ChatRole::User, "t".repeat(100_000)),
            ChatMessage::new(ChatRole::Assistant, "ok"),
        ];

        prune_message_history(&mut messages);

        assert!(history_char_len(&messages) <= MAX_HISTORY_CHARS);
        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        // 100k fits under the 200k budget with prefix retained.
        assert!(
            messages
                .iter()
                .any(|message| message.content.chars().count() == 100_000)
        );
    }
}
