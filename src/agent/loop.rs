use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::limits::{LimitExceeded, LimitsConfig, RunMetrics};
use crate::logging::Loggers;
use crate::mcp::{stdio_client::McpError, ToolExecutor};
use crate::output::{RunOutput, StopReason, UsageReport};
use crate::provider::{openai_compat::ProviderError, ChatMessage, ChatRole, ModelClient};

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider failure: {0}")]
    Provider(#[from] ProviderError),
    #[error("tool failure: {0}")]
    Mcp(#[from] McpError),
    #[error("failed to decode model directive: {0}")]
    DirectiveDecode(#[from] serde_json::Error),
    #[error("internal logging failure: {0}")]
    Logging(String),
}

#[derive(Clone)]
pub struct AgentRunRequest {
    pub prompt: String,
    pub system_prompt: String,
    pub input_files_context: String,
    pub limits: LimitsConfig,
    pub allowed_tools: HashSet<String>,
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

    #[allow(clippy::too_many_lines)]
    async fn run_inner(&self, request: AgentRunRequest) -> Result<RunOutput, AgentError> {
        let available_tools = self.tools.available_tools();
        let user_message = build_user_message(
            &request.prompt,
            &request.input_files_context,
            &available_tools,
        );
        let mut messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: request.system_prompt.clone(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: user_message,
            },
        ];
        let mut metrics = RunMetrics::new();
        let mut final_result = String::new();
        let mut stop_reason = StopReason::LimitReached;

        loop {
            match metrics.pre_step_check(&request.limits) {
                Ok(()) => {}
                Err(limit) => {
                    final_result = format!("Execution stopped due to limit: {}", limit_name(limit));
                    stop_reason = StopReason::LimitReached;
                    break;
                }
            }
            metrics.begin_iteration();

            let completion = self.model.complete(&messages).await?;
            metrics.add_tokens(completion.usage);
            if metrics.tokens_limit_hit(&request.limits) {
                final_result = "Execution stopped due to limit: max_tokens".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }
            if metrics.duration_limit_hit(&request.limits) {
                final_result = "Execution stopped due to limit: max_duration_sec".to_string();
                stop_reason = StopReason::LimitReached;
                break;
            }

            if let Some(ai_log) = &self.loggers.ai {
                ai_log
                    .write_event(
                        "ai_completion",
                        &json!({
                            "iteration": metrics.iterations(),
                            "content": completion.content,
                            "usage": completion.usage,
                        }),
                    )
                    .await
                    .map_err(|err| AgentError::Logging(err.to_string()))?;
            }

            let directive = parse_directive(&completion.content);
            let directive = match directive {
                Ok(v) => v,
                Err(_) => ModelDirective {
                    done: true,
                    thought: Some("fallback raw output".to_string()),
                    tool_calls: Vec::new(),
                    result: Some(completion.content.clone()),
                },
            };

            messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: completion.content,
            });

            for tool_call in directive.tool_calls {
                let qualified_tool = format!("{}.{}", tool_call.server, tool_call.tool);
                if !request.allowed_tools.is_empty()
                    && !request.allowed_tools.contains(&tool_call.tool)
                    && !request.allowed_tools.contains(&qualified_tool)
                {
                    let warning = json!({
                        "tool_call": tool_call,
                        "error": "tool is not allowed by skills policy"
                    });
                    messages.push(ChatMessage {
                        role: ChatRole::User,
                        content: warning.to_string(),
                    });
                    continue;
                }

                let tool_response = self
                    .tools
                    .call_tool(
                        &tool_call.server,
                        &tool_call.tool,
                        tool_call.arguments.clone(),
                    )
                    .await?;
                if metrics.duration_limit_hit(&request.limits) {
                    final_result = "Execution stopped due to limit: max_duration_sec".to_string();
                    stop_reason = StopReason::LimitReached;
                    break;
                }
                messages.push(ChatMessage {
                    role: ChatRole::User,
                    content: json!({
                        "tool_result": {
                            "server": tool_call.server,
                            "tool": tool_call.tool,
                            "result": tool_response
                        }
                    })
                    .to_string(),
                });
            }

            if stop_reason == StopReason::LimitReached && !final_result.is_empty() {
                break;
            }

            if directive.done {
                final_result = directive
                    .result
                    .unwrap_or_else(|| "Agent marked done without explicit result".to_string());
                stop_reason = StopReason::GoalReached;
                break;
            }
        }

        let tokens = metrics.tokens();
        Ok(RunOutput {
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
        })
    }
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
}
