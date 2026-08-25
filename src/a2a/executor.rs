//! [`a2a_server::AgentExecutor`] that runs one Kuibysheff prompt per A2A task.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use a2a::event::StreamResponse;
use a2a::{
    new_artifact_id, new_message_id, A2AError, Artifact, Message, Part, PartContent, Role, Task,
    TaskState, TaskStatus, TaskStatusUpdateEvent,
};
use a2a_server::{AgentExecutor, ExecutorContext};
use futures::stream::{self, BoxStream};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

use crate::agent::{AgentEventTx, RunCancel};
use crate::app::{run_agent_prompt, AgentPromptArgs};
use crate::billing::Money;
use crate::output::{RunOutput, StopReason};
use crate::project_paths::{resolve_agent_identity, ResolvedAgentPaths};

/// Maximum concurrent A2A agent runs (mirrors ACP session cap).
pub const MAX_IN_FLIGHT_TASKS: usize = 256;

/// Generic message returned to peers when internal wiring fails.
const PEER_RUN_FAILED_MSG: &str = "agent run failed";

/// Async runner for one prompt turn (test seam over [`run_agent_prompt`]).
pub trait TaskRunner: Send + Sync + 'static {
    /// Execute one agent turn with the given wiring.
    fn run(
        &self,
        args: AgentPromptArgs,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<RunOutput>> + Send>>;
}

/// Production runner that calls [`run_agent_prompt`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EngineTaskRunner;

impl TaskRunner for EngineTaskRunner {
    fn run(
        &self,
        args: AgentPromptArgs,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<RunOutput>> + Send>> {
        Box::pin(run_agent_prompt(args))
    }
}

/// Profile identity and limit overrides shared across A2A tasks.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub paths: ResolvedAgentPaths,
    pub max_iterations: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_duration_sec: Option<u64>,
    pub max_cost: Option<Money>,
    pub save_chat_history: bool,
}

impl ExecutorConfig {
    /// Build config from CLI `a2a` arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when the agent id / home path is invalid.
    pub fn from_a2a_args(args: &crate::cli::A2aArgs) -> anyhow::Result<Self> {
        let paths = resolve_agent_identity(
            &args.identity.project_root,
            &args.identity.agent,
            args.home.as_deref(),
        )?;
        Ok(Self {
            paths,
            max_iterations: args.max_iterations,
            max_tokens: args.max_tokens,
            max_duration_sec: args.max_duration_sec,
            max_cost: args.max_cost.clone(),
            save_chat_history: args.save_chat_history,
        })
    }
}

/// A2A executor that maps each message to one Kuibysheff `run_agent_prompt`.
pub struct KuibysheffExecutor<R: TaskRunner = EngineTaskRunner> {
    config: Arc<ExecutorConfig>,
    runner: Arc<R>,
    cancels: Arc<Mutex<HashMap<String, RunCancel>>>,
    in_flight: Arc<Semaphore>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl KuibysheffExecutor<EngineTaskRunner> {
    /// Create an executor using the real agent engine.
    #[must_use]
    pub fn new(config: ExecutorConfig) -> Self {
        Self::with_runner(config, EngineTaskRunner)
    }
}

impl<R: TaskRunner> KuibysheffExecutor<R> {
    /// Create an executor with a custom [`TaskRunner`] (tests).
    #[must_use]
    pub fn with_runner(config: ExecutorConfig, runner: R) -> Self {
        Self {
            config: Arc::new(config),
            runner: Arc::new(runner),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(Semaphore::new(MAX_IN_FLIGHT_TASKS)),
            tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Shared cancel registry (for graceful shutdown).
    #[must_use]
    pub fn cancels(&self) -> Arc<Mutex<HashMap<String, RunCancel>>> {
        Arc::clone(&self.cancels)
    }

    /// In-flight task join set (for graceful shutdown).
    #[must_use]
    pub fn tasks(&self) -> Arc<Mutex<Vec<JoinHandle<()>>>> {
        Arc::clone(&self.tasks)
    }

    fn extract_prompt(message: &Option<Message>) -> Result<String, A2AError> {
        let Some(message) = message else {
            return Err(A2AError::invalid_params("message is required"));
        };
        let mut texts = Vec::new();
        for part in &message.parts {
            match &part.content {
                PartContent::Text(text) => texts.push(text.clone()),
                PartContent::Raw(_) | PartContent::Url(_) | PartContent::Data(_) => {
                    return Err(A2AError::content_type_not_supported());
                }
            }
        }
        let prompt = texts.join("\n");
        if prompt.trim().is_empty() {
            return Err(A2AError::invalid_params("message has no text parts"));
        }
        Ok(prompt)
    }

    fn prompt_args(config: &ExecutorConfig, prompt: String, cancel: RunCancel) -> AgentPromptArgs {
        let paths = &config.paths;
        AgentPromptArgs {
            config: paths.config.clone(),
            settings_dir: paths.settings_dir.clone(),
            home: paths.home.clone(),
            project_root: Some(paths.project_root.clone()),
            agent_id: paths.agent_id.clone(),
            prompt,
            run_id: None,
            files: Vec::new(),
            max_iterations: config.max_iterations,
            max_tokens: config.max_tokens,
            max_duration_sec: config.max_duration_sec,
            max_cost: config.max_cost.clone(),
            save_chat_history: config.save_chat_history,
            cancel,
            events: AgentEventTx::noop(),
        }
    }

    fn final_task(
        task_id: String,
        context_id: String,
        state: TaskState,
        result_text: String,
        usage: Option<serde_json::Value>,
    ) -> Task {
        let mut metadata = HashMap::new();
        if let Some(usage) = usage {
            metadata.insert("kuibysheff.usage".to_string(), usage);
        }
        Task {
            id: task_id.clone(),
            context_id: context_id.clone(),
            status: TaskStatus {
                state,
                message: Some(Message {
                    role: Role::Agent,
                    message_id: new_message_id(),
                    task_id: Some(task_id),
                    context_id: Some(context_id),
                    parts: vec![Part::text(result_text.clone())],
                    metadata: None,
                    extensions: None,
                    reference_task_ids: None,
                }),
                timestamp: Some(chrono::Utc::now()),
            },
            artifacts: Some(vec![Artifact {
                artifact_id: new_artifact_id(),
                name: Some("result".into()),
                description: Some("Agent run result".into()),
                parts: vec![Part::text(result_text)],
                metadata: None,
                extensions: None,
            }]),
            history: None,
            metadata: if metadata.is_empty() {
                None
            } else {
                Some(metadata)
            },
        }
    }
}

impl<R: TaskRunner> AgentExecutor for KuibysheffExecutor<R> {
    fn execute(
        &self,
        ctx: ExecutorContext,
    ) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
        let prompt = match Self::extract_prompt(&ctx.message) {
            Ok(p) => p,
            Err(err) => return Box::pin(stream::once(async move { Err(err) })),
        };

        let in_flight = Arc::clone(&self.in_flight);
        let permit = match in_flight.try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return Box::pin(stream::once(async move {
                    Err(A2AError::invalid_params(format!(
                        "too many in-flight tasks (max {MAX_IN_FLIGHT_TASKS})"
                    )))
                }));
            }
        };

        let task_id = ctx.task_id.clone();
        let context_id = ctx.context_id.clone();
        let cancel = RunCancel::new();

        {
            let mut map = self
                .cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            map.insert(task_id.clone(), cancel.clone());
        }

        if cancel.is_cancelled() {
            self.cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&task_id);
            drop(permit);
            return Box::pin(stream::once(async move {
                Ok(StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                    task_id,
                    context_id,
                    status: TaskStatus {
                        state: TaskState::Canceled,
                        message: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                    metadata: None,
                }))
            }));
        }

        let prompt_args = Self::prompt_args(&self.config, prompt, cancel.clone());
        let cancels = Arc::clone(&self.cancels);
        let runner = Arc::clone(&self.runner);
        let task_handles = Arc::clone(&self.tasks);

        let (tx, rx) = mpsc::channel(8);
        let handle = tokio::spawn(async move {
            let _permit = permit;

            let working = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task_id.clone(),
                context_id: context_id.clone(),
                status: TaskStatus {
                    state: TaskState::Working,
                    message: None,
                    timestamp: Some(chrono::Utc::now()),
                },
                metadata: None,
            });
            if tx.send(Ok(working)).await.is_err() {
                cancels
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&task_id);
                return;
            }

            if cancel.is_cancelled() {
                cancels
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&task_id);
                let task = KuibysheffExecutor::<R>::final_task(
                    task_id,
                    context_id,
                    TaskState::Canceled,
                    String::new(),
                    None,
                );
                let _ = tx.send(Ok(StreamResponse::Task(task))).await;
                return;
            }

            info!(task_id = %task_id, "A2A task starting agent run");
            let run_result = runner.run(prompt_args).await;
            cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&task_id);

            let (state, text, usage) = match run_result {
                Ok(output) => map_run_output(output, &cancel),
                Err(err) => {
                    warn!(?err, task_id = %task_id, "A2A task wiring failed");
                    (
                        if cancel.is_cancelled() {
                            TaskState::Canceled
                        } else {
                            TaskState::Failed
                        },
                        PEER_RUN_FAILED_MSG.to_string(),
                        None,
                    )
                }
            };

            let task = KuibysheffExecutor::<R>::final_task(task_id, context_id, state, text, usage);
            let _ = tx.send(Ok(StreamResponse::Task(task))).await;
        });
        task_handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(handle);

        Box::pin(ReceiverStream::new(rx))
    }

    fn cancel(&self, ctx: ExecutorContext) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
        let task_id = ctx.task_id.clone();
        let context_id = ctx.context_id.clone();
        let cancels = Arc::clone(&self.cancels);

        Box::pin(stream::once(async move {
            let found = {
                let map = cancels
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(cancel) = map.get(&task_id) {
                    cancel.cancel();
                    true
                } else {
                    false
                }
            };
            if !found {
                return Err(A2AError::invalid_params(format!(
                    "unknown task id `{task_id}`"
                )));
            }
            Ok(StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id,
                context_id,
                status: TaskStatus {
                    state: TaskState::Canceled,
                    message: None,
                    timestamp: Some(chrono::Utc::now()),
                },
                metadata: None,
            }))
        }))
    }
}

fn map_run_output(
    output: RunOutput,
    cancel: &RunCancel,
) -> (TaskState, String, Option<serde_json::Value>) {
    let usage = serde_json::to_value(&output.usage).ok();
    if cancel.is_cancelled() {
        return (TaskState::Canceled, output.result, usage);
    }
    let state = match output.stop_reason {
        StopReason::GoalReached | StopReason::LimitReached => TaskState::Completed,
        StopReason::Error => TaskState::Failed,
    };
    (state, output.result, usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone)]
    struct FakeRunner {
        delay_ms: u64,
        fail: bool,
        saw_cancel: Arc<AtomicBool>,
    }

    impl TaskRunner for FakeRunner {
        fn run(
            &self,
            args: AgentPromptArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<RunOutput>> + Send>> {
            let delay = self.delay_ms;
            let fail = self.fail;
            let saw = Arc::clone(&self.saw_cancel);
            Box::pin(async move {
                if delay > 0 {
                    tokio::select! {
                        () = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {}
                        () = args.cancel.token().cancelled() => {
                            saw.store(true, Ordering::SeqCst);
                            return Ok(RunOutput {
                                run_id: "fake".into(),
                                result: "cancelled".into(),
                                usage: Default::default(),
                                stop_reason: StopReason::Error,
                                logs: Default::default(),
                            });
                        }
                    }
                }
                if fail {
                    return Ok(RunOutput {
                        run_id: "fake".into(),
                        result: "boom".into(),
                        usage: Default::default(),
                        stop_reason: StopReason::Error,
                        logs: Default::default(),
                    });
                }
                Ok(RunOutput {
                    run_id: "fake".into(),
                    result: format!("echo:{}", args.prompt),
                    usage: Default::default(),
                    stop_reason: StopReason::GoalReached,
                    logs: Default::default(),
                })
            })
        }
    }

    fn test_config() -> ExecutorConfig {
        ExecutorConfig {
            paths: ResolvedAgentPaths {
                project_root: PathBuf::from("/proj"),
                agent_id: "demo".into(),
                profile_dir: PathBuf::from("/proj/.kuibysheff/protected/agents/demo"),
                settings_dir: PathBuf::from("/proj/.kuibysheff/protected/agents/demo"),
                config: PathBuf::from("/proj/.kuibysheff/protected/agents/demo/agent-config.yaml"),
                home: PathBuf::from("/proj/.kuibysheff/homes/demo"),
            },
            max_iterations: None,
            max_tokens: None,
            max_duration_sec: None,
            max_cost: None,
            save_chat_history: false,
        }
    }

    fn exec_ctx(task_id: &str, message: Option<Message>) -> ExecutorContext {
        ExecutorContext {
            message,
            task_id: task_id.into(),
            stored_task: None,
            context_id: "c1".into(),
            metadata: None,
            user: None,
            service_params: HashMap::new(),
            tenant: None,
        }
    }

    #[tokio::test]
    async fn execute_completes_with_text() {
        let exec = KuibysheffExecutor::with_runner(
            test_config(),
            FakeRunner {
                delay_ms: 0,
                fail: false,
                saw_cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        let mut stream = exec.execute(exec_ctx(
            "t1",
            Some(Message::new(Role::User, vec![Part::text("hi")])),
        ));
        let first = stream.next().await.expect("working").expect("ok");
        assert!(matches!(
            first,
            StreamResponse::StatusUpdate(ref u) if u.status.state == TaskState::Working
        ));
        let second = stream.next().await.expect("task").expect("ok");
        match second {
            StreamResponse::Task(task) => {
                assert_eq!(task.status.state, TaskState::Completed);
                let msg = task.status.message.expect("agent message");
                assert_eq!(msg.text(), Some("echo:hi"));
            }
            other => panic!("expected Task, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_non_text_parts() {
        let exec = KuibysheffExecutor::with_runner(
            test_config(),
            FakeRunner {
                delay_ms: 0,
                fail: false,
                saw_cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        let mut stream = exec.execute(exec_ctx(
            "t1",
            Some(Message::new(
                Role::User,
                vec![Part::data(serde_json::json!({}))],
            )),
        ));
        let err = stream.next().await.expect("err").expect_err("content type");
        assert_eq!(
            err.code,
            a2a::errors::error_code::CONTENT_TYPE_NOT_SUPPORTED
        );
    }

    #[tokio::test]
    async fn cancel_before_run_stops_task() {
        let saw_cancel = Arc::new(AtomicBool::new(false));
        let exec = KuibysheffExecutor::with_runner(
            test_config(),
            FakeRunner {
                delay_ms: 500,
                fail: false,
                saw_cancel: Arc::clone(&saw_cancel),
            },
        );

        let mut run_stream = exec.execute(exec_ctx(
            "t-cancel",
            Some(Message::new(Role::User, vec![Part::text("hi")])),
        ));

        let mut cancel_stream = exec.cancel(exec_ctx("t-cancel", None));
        let cancel_evt = cancel_stream.next().await.expect("cancel").expect("ok");
        assert!(matches!(
            cancel_evt,
            StreamResponse::StatusUpdate(ref u) if u.status.state == TaskState::Canceled
        ));

        let working = run_stream.next().await.expect("working").expect("ok");
        assert!(matches!(
            working,
            StreamResponse::StatusUpdate(ref u) if u.status.state == TaskState::Working
        ));
        let final_evt = run_stream.next().await.expect("final").expect("ok");
        match final_evt {
            StreamResponse::Task(task) => {
                assert_eq!(task.status.state, TaskState::Canceled);
            }
            other => panic!("expected Task, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_unknown_task_returns_error() {
        let exec = KuibysheffExecutor::with_runner(
            test_config(),
            FakeRunner {
                delay_ms: 0,
                fail: false,
                saw_cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        let mut stream = exec.cancel(exec_ctx("missing", None));
        let err = stream.next().await.expect("err").expect_err("unknown");
        assert!(err.message.contains("unknown task id"));
    }

    #[tokio::test]
    async fn wiring_failure_returns_generic_peer_message() {
        struct FailingRunner;

        impl TaskRunner for FailingRunner {
            fn run(
                &self,
                _args: AgentPromptArgs,
            ) -> Pin<Box<dyn Future<Output = anyhow::Result<RunOutput>> + Send>> {
                Box::pin(async { anyhow::bail!("secret internal path /etc/shadow failed") })
            }
        }

        let exec = KuibysheffExecutor::with_runner(test_config(), FailingRunner);
        let mut stream = exec.execute(exec_ctx(
            "t-fail",
            Some(Message::new(Role::User, vec![Part::text("hi")])),
        ));
        let _ = stream.next().await.expect("working").expect("ok");
        let final_evt = stream.next().await.expect("final").expect("ok");
        match final_evt {
            StreamResponse::Task(task) => {
                assert_eq!(task.status.state, TaskState::Failed);
                let msg = task.status.message.expect("msg");
                assert_eq!(msg.text(), Some(PEER_RUN_FAILED_MSG));
                assert!(!msg.text().unwrap().contains("shadow"));
            }
            other => panic!("expected Task, got {other:?}"),
        }
    }
}
