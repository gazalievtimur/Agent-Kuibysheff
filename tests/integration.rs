use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use agent_Kuibyshev::agent::{AgentEngine, AgentRunRequest};
use agent_Kuibyshev::limits::{LimitsConfig, TokenUsage};
use agent_Kuibyshev::config::{LogSinkConfig, LoggingConfig};
use agent_Kuibyshev::logging::Loggers;
use agent_Kuibyshev::mcp::{stdio_client::McpError, ToolExecutor};
use agent_Kuibyshev::output::StopReason;
use agent_Kuibyshev::provider::openai_compat::ProviderError;
use agent_Kuibyshev::provider::{ChatMessage, ModelClient, ModelResponse};
use agent_Kuibyshev::tools::fs_home::HomeFs;
use agent_Kuibyshev::tools::local_tools::LocalTools;
use agent_Kuibyshev::tools::CompositeToolExecutor;

struct FakeModel {
    responses: Mutex<VecDeque<ModelResponse>>,
}

#[async_trait]
impl ModelClient for FakeModel {
    async fn complete(&self, _messages: &[ChatMessage]) -> Result<ModelResponse, ProviderError> {
        let mut guard = self.responses.lock().await;
        guard.pop_front().ok_or(ProviderError::EmptyChoices)
    }
}

struct FakeTools;

#[async_trait]
impl ToolExecutor for FakeTools {
    async fn call_tool(
        &self,
        _server: &str,
        _tool: &str,
        _arguments: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        Ok(serde_json::json!({"ok": true}))
    }

    fn available_tools(&self) -> Vec<String> {
        vec!["local.echo".to_string()]
    }
}

#[tokio::test]
async fn run_finishes_when_model_marks_done() {
    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![ModelResponse {
            content: r#"{"done":true,"thought":"done","tool_calls":[],"result":"task completed"}"#
                .to_string(),
            usage: TokenUsage {
                prompt_tokens: 4,
                completion_tokens: 3,
                total_tokens: 7,
            },
        }])),
    };

    let engine = AgentEngine::new(Arc::new(model), Arc::new(FakeTools), Loggers::default());
    let output = engine
        .run(AgentRunRequest {
            prompt: "finish".to_string(),
            system_prompt: "system".to_string(),
            input_files_context: String::new(),
            limits: LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
            allowed_tools: None,
        })
        .await;

    assert_eq!(output.stop_reason, StopReason::GoalReached);
    assert_eq!(output.result, "task completed");
    assert_eq!(output.usage.iterations, 1);
    assert_eq!(output.usage.total_tokens, 7);
}

#[tokio::test]
async fn run_stops_on_iteration_limit() {
    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![
            ModelResponse {
                content: r#"{"done":false,"thought":"step1","tool_calls":[],"result":null}"#
                    .to_string(),
                usage: TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
            },
            ModelResponse {
                content: r#"{"done":false,"thought":"step2","tool_calls":[],"result":null}"#
                    .to_string(),
                usage: TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
            },
        ])),
    };

    let engine = AgentEngine::new(Arc::new(model), Arc::new(FakeTools), Loggers::default());
    let output = engine
        .run(AgentRunRequest {
            prompt: "never done".to_string(),
            system_prompt: "system".to_string(),
            input_files_context: String::new(),
            limits: LimitsConfig {
                max_iterations: 2,
                max_tokens: 100,
                max_duration_sec: 60,
            },
            allowed_tools: None,
        })
        .await;

    assert_eq!(output.stop_reason, StopReason::LimitReached);
    assert!(output.result.contains("max_iterations"));
    assert_eq!(output.usage.iterations, 2);
}

#[tokio::test]
async fn model_can_write_an_artifact_inside_home() {
    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![
            ModelResponse {
                content: r#"{"done":false,"thought":"write","tool_calls":[{"server":"home","tool":"write","arguments":{"path":"out/result.txt","content":"ready"}}],"result":null}"#.to_string(),
                usage: TokenUsage::default(),
            },
            ModelResponse {
                content:
                    r#"{"done":true,"thought":"done","tool_calls":[],"result":"artifact created"}"#
                        .to_string(),
                usage: TokenUsage::default(),
            },
        ])),
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let home = HomeFs::new(dir.path()).await.expect("home");
    let local_tools = LocalTools::new(dir.path()).await.expect("local tools");
    let tools = CompositeToolExecutor::new(home, local_tools, Arc::new(FakeTools));
    let engine = AgentEngine::new(Arc::new(model), Arc::new(tools), Loggers::default());

    let output = engine
        .run(AgentRunRequest {
            prompt: "write an artifact".to_string(),
            system_prompt: "system".to_string(),
            input_files_context: String::new(),
            limits: LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
            allowed_tools: Some(HashSet::from(["home.write".to_string()])),
        })
        .await;

    assert_eq!(output.stop_reason, StopReason::GoalReached);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/result.txt")).expect("read artifact"),
        "ready"
    );
}

#[tokio::test]
async fn run_retries_after_invalid_model_json() {
    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![
            ModelResponse {
                content: "Here is my answer in prose, not JSON.".to_string(),
                usage: TokenUsage::default(),
            },
            ModelResponse {
                content: r#"{"done":true,"thought":"fixed","tool_calls":[],"result":"recovered"}"#
                    .to_string(),
                usage: TokenUsage::default(),
            },
        ])),
    };

    let engine = AgentEngine::new(Arc::new(model), Arc::new(FakeTools), Loggers::default());
    let output = engine
        .run(AgentRunRequest {
            prompt: "finish".to_string(),
            system_prompt: "system".to_string(),
            input_files_context: String::new(),
            limits: LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
            allowed_tools: None,
        })
        .await;

    assert_eq!(output.stop_reason, StopReason::GoalReached);
    assert_eq!(output.result, "recovered");
    assert_eq!(output.usage.iterations, 2);
}

#[tokio::test]
async fn run_saves_full_chat_history_when_enabled() {
    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![ModelResponse {
            content: r#"{"done":true,"thought":"done","tool_calls":[],"result":"saved"}"#
                .to_string(),
            usage: TokenUsage {
                prompt_tokens: 2,
                completion_tokens: 1,
                total_tokens: 3,
            },
        }])),
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let loggers = Loggers::from_config(&LoggingConfig {
        enable_ai_log: false,
        enable_mcp_log: false,
        enable_chat_history: true,
        output_dir: Some(dir.path().to_path_buf()),
        sink: LogSinkConfig::default(),
    })
    .await
    .expect("loggers");

    let engine = AgentEngine::new(Arc::new(model), Arc::new(FakeTools), loggers);
    let output = engine
        .run(AgentRunRequest {
            prompt: "save chat".to_string(),
            system_prompt: "system".to_string(),
            input_files_context: String::new(),
            limits: LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
            allowed_tools: None,
        })
        .await;

    let chat_log = output.logs.chat_log.expect("chat log path");
    let contents = std::fs::read_to_string(chat_log).expect("chat history file");
    assert!(contents.contains("\"role\": \"system\""));
    assert!(contents.contains("\"role\": \"assistant\""));
    assert!(contents.contains("saved"));
}
