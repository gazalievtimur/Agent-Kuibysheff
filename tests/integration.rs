use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use agent_Kuibyshev::access::{
    CanonicalRoot, EffectiveToolPolicy, HomeFsPolicy, ProgramAlias, QualifiedTool,
    ResolvedAccessPolicy, ResolvedProgramPolicy, WorkspaceFsPolicy,
};
use agent_Kuibyshev::agent::{AgentEngine, AgentRunRequest, RunCancel};
use agent_Kuibyshev::config::{LogSinkConfig, LoggingConfig};
use agent_Kuibyshev::limits::{LimitsConfig, TokenUsage};
use agent_Kuibyshev::logging::Loggers;
use agent_Kuibyshev::output::StopReason;
use agent_Kuibyshev::provider::{ChatMessage, Error as ProviderError, ModelClient, ModelResponse};
use agent_Kuibyshev::tool_api::ToolExecutor;
use agent_Kuibyshev::tools::fs_home::HomeFs;
use agent_Kuibyshev::tools::local_tools::LocalTools;
use agent_Kuibyshev::tools::ToolError;
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
        history: agent_Kuibyshev::config::ProviderHistoryConfig::default(),
        cancel: RunCancel::new(),
    }
}

async fn make_tools_with_runner(
    dir: &Path,
    home_policy: HomeFsPolicy,
    runner: Arc<agent_Kuibyshev::sandbox::SandboxRunner>,
) -> Arc<CompositeToolExecutor> {
    let home = HomeFs::new(dir, home_policy, runner, RunCancel::new())
        .await
        .expect("home");
    let local_tools = LocalTools::new(dir, WorkspaceFsPolicy::legacy())
        .await
        .expect("local tools");
    Arc::new(CompositeToolExecutor::new(
        home,
        local_tools,
        Arc::new(FakeTools),
    ))
}

async fn make_legacy_tools(dir: &Path) -> Arc<CompositeToolExecutor> {
    make_tools_with_runner(
        dir,
        HomeFsPolicy::legacy(),
        Arc::new(agent_Kuibyshev::sandbox::SandboxRunner::platform_default()),
    )
    .await
}

fn policy_executor_for(
    composite: Arc<CompositeToolExecutor>,
    access: &ResolvedAccessPolicy,
    tools: &[&str],
) -> Arc<PolicyToolExecutor> {
    let allowed: BTreeSet<QualifiedTool> = tools
        .iter()
        .map(|tool| QualifiedTool::parse(tool).expect("valid tool"))
        .collect();
    let policy = EffectiveToolPolicy::compile(access, &allowed, BTreeSet::new());
    Arc::new(PolicyToolExecutor::new(composite, policy))
}

fn home_policy_with_program(alias: &str, exe: &Path) -> HomeFsPolicy {
    let mut home_policy = HomeFsPolicy::legacy();
    let executable = CanonicalRoot::canonicalize(exe).expect("canonicalize");
    let mut programs = BTreeMap::new();
    programs.insert(
        ProgramAlias::parse(alias).unwrap(),
        ResolvedProgramPolicy {
            alias: ProgramAlias::parse(alias).unwrap(),
            executable,
            runtime_read_roots: Vec::new(),
            inherit_env: Vec::new(),
            allow_children: false,
        },
    );
    home_policy.programs = programs;
    home_policy
}

fn prepare_fixture_exe(dir: &Path) -> std::path::PathBuf {
    let fixture = std::path::PathBuf::from(env!("CARGO_BIN_EXE_sandbox_e2e_fixture"));
    #[cfg(windows)]
    let local_exe = dir.join("sandbox_e2e_fixture.exe");
    #[cfg(not(windows))]
    let local_exe = dir.join("sandbox_e2e_fixture");
    std::fs::copy(&fixture, &local_exe).expect("copy fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&local_exe).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&local_exe, perms).expect("chmod");
    }
    local_exe
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
    let composite = make_legacy_tools(dir.path()).await;
    let tools = policy_executor_for(composite, &ResolvedAccessPolicy::legacy(), &["home.write"]);
    let engine = AgentEngine::new(Arc::new(model), tools, Loggers::default());

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
    let composite = make_legacy_tools(dir.path()).await;
    let tools = policy_executor_for(composite, &ResolvedAccessPolicy::legacy(), &["home.read"]);
    let engine = AgentEngine::new(Arc::new(model), tools, Loggers::default());

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
    use agent_Kuibyshev::access::{
        resolve_access_policy, AccessPolicyConfig, FilesystemPolicyConfig, HomeFsPolicyConfig,
        RunPolicyConfig, ToolsPolicyConfig,
    };

    let runner = Arc::new(agent_Kuibyshev::sandbox::SandboxRunner::platform_default());
    if runner.probe().is_err() {
        return;
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let local_exe = prepare_fixture_exe(dir.path());
    let home_policy = home_policy_with_program("fixture", &local_exe);

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

    let composite = make_tools_with_runner(dir.path(), home_policy, runner).await;

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
        ..AccessPolicyConfig::default()
    };
    let access = resolve_access_policy(Some(&access_cfg), dir.path()).expect("access policy");
    let tools = policy_executor_for(composite, &access, &["home.run"]);
    let engine = AgentEngine::new(Arc::new(model), tools, Loggers::default());

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
