use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use agent_Kuibyshev::access::{EffectiveToolPolicy, QualifiedTool, ResolvedAccessPolicy};
use agent_Kuibyshev::agent::{AgentEngine, AgentRunRequest};
use agent_Kuibyshev::config::{LogSinkConfig, LoggingConfig};
use agent_Kuibyshev::limits::{LimitsConfig, TokenUsage};
use agent_Kuibyshev::logging::Loggers;
use agent_Kuibyshev::mcp::{Error as ToolError, ToolExecutor};
use agent_Kuibyshev::output::StopReason;
use agent_Kuibyshev::provider::{ChatMessage, Error as ProviderError, ModelClient, ModelResponse};
use agent_Kuibyshev::tools::fs_home::HomeFs;
use agent_Kuibyshev::tools::local_tools::LocalTools;
use agent_Kuibyshev::tools::{CompositeToolExecutor, PolicyToolExecutor};

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
    ) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"ok": true}))
    }

    fn available_tools(&self) -> Vec<String> {
        vec!["local.echo".to_string()]
    }
}

fn request(prompt: &str, limits: LimitsConfig) -> AgentRunRequest {
    AgentRunRequest {
        prompt: prompt.to_string(),
        system_prompt: "system".to_string(),
        input_files_context: String::new(),
        limits,
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
        .run(request(
            "finish",
            LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
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
        .run(request(
            "never done",
            LimitsConfig {
                max_iterations: 2,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
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
    let home = HomeFs::new(
        dir.path(),
        agent_Kuibyshev::access::HomeFsPolicy::legacy(),
        Arc::new(agent_Kuibyshev::sandbox::SandboxRunner::platform_default()),
    )
    .await
    .expect("home");
    let local_tools = LocalTools::new(
        dir.path(),
        agent_Kuibyshev::access::WorkspaceFsPolicy::legacy(),
    )
    .await
    .expect("local tools");
    let composite = CompositeToolExecutor::new(home, local_tools, Arc::new(FakeTools));
    let policy = EffectiveToolPolicy::compile(
        &ResolvedAccessPolicy::legacy(),
        &BTreeSet::from([QualifiedTool::parse("home.write").unwrap()]),
        BTreeSet::new(),
    );
    let tools = PolicyToolExecutor::new(Arc::new(composite), policy);
    let engine = AgentEngine::new(Arc::new(model), Arc::new(tools), Loggers::default());

    let output = engine
        .run(request(
            "write an artifact",
            LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
        .await;

    assert_eq!(output.stop_reason, StopReason::GoalReached);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/result.txt")).expect("read artifact"),
        "ready"
    );
}

#[tokio::test]
async fn denied_tool_is_returned_as_tool_result_error() {
    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![
            ModelResponse {
                content: r#"{"done":false,"thought":"try denied","tool_calls":[{"server":"home","tool":"write","arguments":{"path":"out/x.txt","content":"no"}}],"result":null}"#.to_string(),
                usage: TokenUsage::default(),
            },
            ModelResponse {
                content: r#"{"done":true,"thought":"gave up","tool_calls":[],"result":"denied visible"}"#
                    .to_string(),
                usage: TokenUsage::default(),
            },
        ])),
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let home = HomeFs::new(
        dir.path(),
        agent_Kuibyshev::access::HomeFsPolicy::legacy(),
        Arc::new(agent_Kuibyshev::sandbox::SandboxRunner::platform_default()),
    )
    .await
    .expect("home");
    let local_tools = LocalTools::new(
        dir.path(),
        agent_Kuibyshev::access::WorkspaceFsPolicy::legacy(),
    )
    .await
    .expect("local tools");
    let composite = CompositeToolExecutor::new(home, local_tools, Arc::new(FakeTools));
    let policy = EffectiveToolPolicy::compile(
        &ResolvedAccessPolicy::legacy(),
        &BTreeSet::from([QualifiedTool::parse("home.read").unwrap()]),
        BTreeSet::new(),
    );
    let tools = PolicyToolExecutor::new(Arc::new(composite), policy);
    let engine = AgentEngine::new(Arc::new(model), Arc::new(tools), Loggers::default());

    let output = engine
        .run(request(
            "deny write",
            LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
        .await;

    assert_eq!(output.stop_reason, StopReason::GoalReached);
    assert!(!dir.path().join("out/x.txt").exists());
    assert_eq!(output.result, "denied visible");
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
#[tokio::test]
async fn model_can_home_run_via_native_sandbox() {
    use agent_Kuibyshev::access::resolve_access_policy;
    use agent_Kuibyshev::config::{
        AccessPolicyConfig, FilesystemPolicyConfig, HomeFsPolicyConfig, RunPolicyConfig,
        ToolsPolicyConfig,
    };

    let runner = agent_Kuibyshev::sandbox::SandboxRunner::platform_default();
    if runner.probe().is_err() {
        return;
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = std::path::PathBuf::from(env!("CARGO_BIN_EXE_sandbox_e2e_fixture"));
    #[cfg(windows)]
    let local_exe = dir.path().join("sandbox_e2e_fixture.exe");
    #[cfg(not(windows))]
    let local_exe = dir.path().join("sandbox_e2e_fixture");
    std::fs::copy(&fixture, &local_exe).expect("copy fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&local_exe).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&local_exe, perms).expect("chmod");
    }

    let mut home_policy = agent_Kuibyshev::access::HomeFsPolicy::legacy();
    let executable =
        agent_Kuibyshev::access::CanonicalRoot::canonicalize(&local_exe).expect("canonicalize");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(
        agent_Kuibyshev::access::ProgramAlias::parse("fixture").unwrap(),
        agent_Kuibyshev::access::ResolvedProgramPolicy {
            alias: agent_Kuibyshev::access::ProgramAlias::parse("fixture").unwrap(),
            executable,
            runtime_read_roots: Vec::new(),
            inherit_env: Vec::new(),
            allow_children: false,
        },
    );
    home_policy.programs = programs;

    let model = FakeModel {
        responses: Mutex::new(VecDeque::from(vec![
            ModelResponse {
                content: r#"{"done":false,"thought":"run","tool_calls":[{"server":"home","tool":"run","arguments":{"program":"fixture","args":["echo","hello-agent-e2e"],"timeout_ms":15000}}],"result":null}"#.to_string(),
                usage: TokenUsage::default(),
            },
            ModelResponse {
                content: r#"{"done":true,"thought":"done","tool_calls":[],"result":"ran ok"}"#
                    .to_string(),
                usage: TokenUsage::default(),
            },
        ])),
    };

    let home = HomeFs::new(dir.path(), home_policy, Arc::new(runner))
        .await
        .expect("home");
    let local_tools = LocalTools::new(
        dir.path(),
        agent_Kuibyshev::access::WorkspaceFsPolicy::legacy(),
    )
    .await
    .expect("local tools");
    let composite = CompositeToolExecutor::new(home, local_tools, Arc::new(FakeTools));

    let access_cfg = AccessPolicyConfig {
        tools: ToolsPolicyConfig {
            builtins: vec!["home.run".into()],
        },
        filesystem: FilesystemPolicyConfig {
            home: HomeFsPolicyConfig {
                read: vec![".".into()],
                write: vec![".".into()],
            },
            workspace: None,
            input_roots: Vec::new(),
        },
        run: RunPolicyConfig {
            programs: Vec::new(),
            max_args: 32,
            max_arg_chars: 4096,
            max_output_chars: 65_536,
            max_timeout_ms: 60_000,
        },
    };
    let access = resolve_access_policy(Some(&access_cfg), dir.path()).expect("access policy");
    let policy = EffectiveToolPolicy::compile(
        &access,
        &BTreeSet::from([QualifiedTool::parse("home.run").unwrap()]),
        BTreeSet::new(),
    );
    let tools = PolicyToolExecutor::new(Arc::new(composite), policy);
    let engine = AgentEngine::new(Arc::new(model), Arc::new(tools), Loggers::default());

    let output = engine
        .run(request(
            "run fixture",
            LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
        .await;

    assert_eq!(output.stop_reason, StopReason::GoalReached);
    assert_eq!(output.result, "ran ok");
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
        .run(request(
            "finish",
            LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
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
        .run(request(
            "save chat",
            LimitsConfig {
                max_iterations: 3,
                max_tokens: 100,
                max_duration_sec: 60,
            },
        ))
        .await;

    let chat_log = output.logs.chat_log.expect("chat log path");
    let contents = std::fs::read_to_string(chat_log).expect("chat history file");
    assert!(contents.contains("\"role\": \"system\""));
    assert!(contents.contains("\"role\": \"assistant\""));
    assert!(contents.contains("saved"));
}
