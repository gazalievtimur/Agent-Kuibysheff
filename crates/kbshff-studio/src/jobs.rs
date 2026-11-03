//! One test run at a time: spawn `kbshff run --progress stderr` and fan out NDJSON.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::interrupt;
use crate::kbshff::Kbshff;

const EVENT_BUFFER: usize = 512;
const BROADCAST_CAPACITY: usize = 256;

/// Shared run registry.
#[derive(Clone, Default)]
pub struct JobManager {
    inner: Arc<Mutex<JobManagerInner>>,
}

#[derive(Default)]
struct JobManagerInner {
    active: Option<ActiveJob>,
    last: Option<Arc<JobHandle>>,
}

struct ActiveJob {
    handle: Arc<JobHandle>,
    cancel: oneshot::Sender<()>,
}

/// Snapshot of a studio run.
pub struct JobHandle {
    pub run_id: String,
    pub agent_id: String,
    state: Mutex<JobState>,
    events_tx: broadcast::Sender<Value>,
}

struct JobState {
    status: JobStatus,
    output: Option<Value>,
    error: Option<String>,
    history: VecDeque<Value>,
}

/// Wire status for the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Body of `POST /api/agents/:id/runs`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StartRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub files: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    pub max_iterations: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_duration_sec: Option<u64>,
    pub max_cost: Option<String>,
    #[serde(default)]
    pub save_chat_history: bool,
}

/// Why a run did not start.
#[derive(Debug)]
pub enum StartJobError {
    Conflict,
    Invalid(String),
}

/// Why cancel failed.
#[derive(Debug)]
pub enum CancelJobError {
    NotFound,
}

impl JobManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `kbshff run` when no other studio run is active.
    ///
    /// # Errors
    ///
    /// Returns [`StartJobError::Conflict`] when a run is already active, or
    /// [`StartJobError::Invalid`] when the prompt is empty or the process cannot start.
    pub async fn start(
        &self,
        kbshff: &Kbshff,
        agent_id: String,
        req: StartRunRequest,
    ) -> Result<Arc<JobHandle>, StartJobError> {
        if req.prompt.trim().is_empty() {
            return Err(StartJobError::Invalid(
                "prompt must not be empty".to_string(),
            ));
        }
        let mut guard = self.inner.lock().await;
        let busy = match &guard.active {
            Some(active) => active.handle.is_running().await,
            None => false,
        };
        if busy {
            return Err(StartJobError::Conflict);
        }

        let run_id = new_run_id();
        let (events_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let handle = Arc::new(JobHandle {
            run_id: run_id.clone(),
            agent_id: agent_id.clone(),
            state: Mutex::new(JobState {
                status: JobStatus::Running,
                output: None,
                error: None,
                history: VecDeque::with_capacity(EVENT_BUFFER),
            }),
            events_tx,
        });
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let args = run_args(kbshff, &agent_id, &run_id, &req);
        let log_dir = studio_run_log_dir(&kbshff.project_root, &agent_id, req.home.as_deref())
            .map_err(StartJobError::Invalid)?;
        let child = spawn_run(&kbshff.bin, &args, &log_dir)
            .await
            .map_err(StartJobError::Invalid)?;
        let worker = Arc::clone(&handle);
        let jobs = Arc::clone(&self.inner);
        tokio::spawn(async move {
            drive_run(worker.clone(), child, cancel_rx).await;
            let mut guard = jobs.lock().await;
            if guard
                .active
                .as_ref()
                .is_some_and(|active| active.handle.run_id == worker.run_id)
            {
                guard.last = Some(worker);
                guard.active = None;
            }
        });
        guard.active = Some(ActiveJob {
            handle: Arc::clone(&handle),
            cancel: cancel_tx,
        });
        Ok(handle)
    }

    pub async fn get(&self, run_id: &str) -> Option<Arc<JobHandle>> {
        let guard = self.inner.lock().await;
        if let Some(active) = &guard.active {
            if active.handle.run_id == run_id {
                return Some(Arc::clone(&active.handle));
            }
        }
        if let Some(last) = &guard.last {
            if last.run_id == run_id {
                return Some(Arc::clone(last));
            }
        }
        None
    }

    /// Ask the child `kbshff run` to cancel.
    ///
    /// # Errors
    ///
    /// Returns [`CancelJobError::NotFound`] when `run_id` is not the active run.
    pub async fn cancel(&self, run_id: &str) -> Result<(), CancelJobError> {
        let mut guard = self.inner.lock().await;
        let Some(active) = guard.active.take() else {
            return Err(CancelJobError::NotFound);
        };
        if active.handle.run_id != run_id {
            guard.active = Some(active);
            return Err(CancelJobError::NotFound);
        }
        let _ = active.cancel.send(());
        guard.active = Some(ActiveJob {
            handle: active.handle,
            cancel: oneshot::channel().0,
        });
        Ok(())
    }
}

impl JobHandle {
    async fn is_running(&self) -> bool {
        matches!(self.state.lock().await.status, JobStatus::Running)
    }

    pub async fn push_event(&self, event: Value) {
        {
            let mut state = self.state.lock().await;
            if state.history.len() >= EVENT_BUFFER {
                state.history.pop_front();
            }
            state.history.push_back(event.clone());
        }
        let _ = self.events_tx.send(event);
    }

    pub async fn history_snapshot(&self) -> Vec<Value> {
        self.state.lock().await.history.iter().cloned().collect()
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events_tx.subscribe()
    }

    pub async fn status_response(&self) -> Value {
        let state = self.state.lock().await;
        serde_json::json!({
            "run_id": self.run_id,
            "agent_id": self.agent_id,
            "status": state.status,
            "output": state.output,
            "error": state.error,
        })
    }

    pub async fn log_path(&self, which: &str) -> Option<PathBuf> {
        let state = self.state.lock().await;
        let logs = state.output.as_ref()?.get("logs")?;
        let key = match which {
            "ai" => "ai_log",
            "mcp" => "mcp_log",
            "system" => "system_log",
            "chat" => "chat_log",
            _ => return None,
        };
        logs.get(key).and_then(Value::as_str).map(PathBuf::from)
    }

    async fn finish(&self, status: JobStatus, output: Option<Value>, error: Option<String>) {
        {
            let mut state = self.state.lock().await;
            state.status = status;
            state.output = output;
            state.error = error;
        }
        self.push_event(serde_json::json!({ "type": "done" })).await;
    }
}

async fn spawn_run(
    bin: &std::path::Path,
    args: &[String],
    log_dir: &std::path::Path,
) -> Result<Child, String> {
    tokio::fs::create_dir_all(log_dir)
        .await
        .map_err(|err| format!("create log dir `{}`: {err}", log_dir.display()))?;
    let mut cmd = Command::new(bin);
    cmd.args(args);
    // Prefer task-local logs over ~/.agent-kuibysheff/logs (see logging.paths AGENT_LOG_DIR).
    cmd.env("AGENT_LOG_DIR", log_dir);
    interrupt::prepare(&mut cmd);
    interrupt::piped(&mut cmd);
    cmd.spawn().map_err(|err| {
        format!(
            "failed to start `{}`: {err}. Install kbshff or pass --kbshff / set KBSHFF_BIN.",
            bin.display()
        )
    })
}

/// `{project}/.kuibysheff/{home|homes/<agent>}/logs` — studio run log base.
fn studio_run_log_dir(
    project_root: &std::path::Path,
    agent: &str,
    home: Option<&std::path::Path>,
) -> Result<PathBuf, String> {
    let home_root = match home {
        None => project_root.join(".kuibysheff").join("homes").join(agent),
        Some(raw) if raw.is_absolute() || raw.has_root() => {
            return Err("home must be relative under .kuibysheff/".to_string());
        }
        Some(raw) => {
            let text = raw.to_string_lossy();
            if text.is_empty()
                || text.contains("..")
                || text.replace('\\', "/").contains("protected/")
                || text.replace('\\', "/") == "protected"
            {
                return Err(format!(
                    "home `{}` must be relative under .kuibysheff/ (not protected/)",
                    raw.display()
                ));
            }
            project_root.join(".kuibysheff").join(raw)
        }
    };
    Ok(home_root.join("logs"))
}

fn run_args(kbshff: &Kbshff, agent: &str, run_id: &str, req: &StartRunRequest) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--project-root".to_string(),
        kbshff.project_root.display().to_string(),
        "--agent".to_string(),
        agent.to_string(),
        "--prompt".to_string(),
        req.prompt.clone(),
        "--run-id".to_string(),
        run_id.to_string(),
        "--progress".to_string(),
        "stderr".to_string(),
    ];
    if let Some(home) = &req.home {
        args.push("--home".to_string());
        args.push(home.display().to_string());
    }
    for file in &req.files {
        args.push("--files".to_string());
        args.push(file.display().to_string());
    }
    if let Some(value) = req.max_iterations {
        args.push("--max-iterations".to_string());
        args.push(value.to_string());
    }
    if let Some(value) = req.max_tokens {
        args.push("--max-tokens".to_string());
        args.push(value.to_string());
    }
    if let Some(value) = req.max_duration_sec {
        args.push("--max-duration-sec".to_string());
        args.push(value.to_string());
    }
    if let Some(value) = &req.max_cost {
        if !value.trim().is_empty() {
            args.push("--max-cost".to_string());
            args.push(value.clone());
        }
    }
    if req.save_chat_history {
        args.push("--save-chat-history".to_string());
    }
    args
}

enum WaitStop {
    Exited(std::io::Result<std::process::ExitStatus>),
    Cancel,
}

async fn drive_run(handle: Arc<JobHandle>, mut child: Child, cancel_rx: oneshot::Receiver<()>) {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stderr_handle = Arc::clone(&handle);
    let stderr_task = tokio::spawn(async move {
        if let Some(pipe) = stderr {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(event) = parse_progress_line(&line) {
                    if event.get("type").and_then(Value::as_str) == Some("done") {
                        continue;
                    }
                    stderr_handle.push_event(event).await;
                }
            }
        }
    });
    let stdout_task = tokio::spawn(async move {
        let Some(mut pipe) = stdout else {
            return String::new();
        };
        let mut buf = Vec::new();
        use tokio::io::AsyncReadExt;
        let _ = pipe.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    });

    let pid = child.id();
    let mut wait = Box::pin(child.wait());
    let stop = tokio::select! {
        status = &mut wait => WaitStop::Exited(status),
        _ = cancel_rx => WaitStop::Cancel,
    };
    let (cancelled, status) = match stop {
        WaitStop::Exited(status) => (false, status),
        WaitStop::Cancel => {
            drop(wait);
            if let Some(pid) = pid {
                if interrupt::interrupt_pid(pid).is_err() {
                    let _ = child.start_kill();
                }
            } else {
                let _ = child.start_kill();
            }
            (true, child.wait().await)
        }
    };
    let stdout = stdout_task.await.unwrap_or_default();
    let _ = stderr_task.await;
    let exit_ok = status.as_ref().is_ok_and(|code| code.success());
    let parsed = serde_json::from_str::<Value>(stdout.trim()).ok();
    if cancelled {
        handle.finish(JobStatus::Cancelled, parsed, None).await;
        return;
    }
    match parsed {
        Some(value) => {
            let failed =
                value.get("stop_reason").and_then(Value::as_str) == Some("error") || !exit_ok;
            let status = if failed {
                JobStatus::Failed
            } else {
                JobStatus::Completed
            };
            handle.finish(status, Some(value), None).await;
        }
        None => {
            let message = if stdout.trim().is_empty() {
                format!(
                    "kbshff exited {}",
                    status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|err| err.to_string())
                )
            } else {
                stdout.trim().to_string()
            };
            handle.finish(JobStatus::Failed, None, Some(message)).await;
        }
    }
}

fn new_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("run-{nanos:x}-{}", std::process::id())
}

/// Parse one stderr progress line. Tracing noise and the terminal `done` marker are skipped
/// by the caller when `type` is `done`.
#[must_use]
pub fn parse_progress_line(line: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let kind = value.get("type").and_then(Value::as_str)?;
    if kind.is_empty() {
        return None;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn progress_parser_keeps_events_and_drops_tracing() {
        let event = parse_progress_line(r#"{"type":"message","text":"hello"}"#).expect("event");
        assert_eq!(event["text"], "hello");
        assert!(parse_progress_line("INFO tracing line").is_none());
        assert!(parse_progress_line("").is_none());
    }

    #[test]
    fn studio_log_dir_defaults_to_agent_home_logs() {
        let root = PathBuf::from("proj");
        let dir = studio_run_log_dir(&root, "demo", None).expect("log dir");
        assert_eq!(
            dir,
            root.join(".kuibysheff")
                .join("homes")
                .join("demo")
                .join("logs")
        );
    }

    #[test]
    fn studio_log_dir_uses_home_override() {
        let root = PathBuf::from("proj");
        let dir =
            studio_run_log_dir(&root, "demo", Some(Path::new("homes/TASK1"))).expect("log dir");
        assert_eq!(
            dir,
            root.join(".kuibysheff")
                .join("homes")
                .join("TASK1")
                .join("logs")
        );
    }

    #[test]
    fn studio_log_dir_rejects_protected_and_absolute() {
        let root = PathBuf::from("proj");
        assert!(studio_run_log_dir(&root, "demo", Some(Path::new("protected/x"))).is_err());
        assert!(studio_run_log_dir(&root, "demo", Some(Path::new("/abs"))).is_err());
    }

    #[tokio::test]
    async fn cancel_unknown_run_is_not_found() {
        let manager = JobManager::new();
        let err = manager.cancel("missing").await.unwrap_err();
        assert!(matches!(err, CancelJobError::NotFound));
    }
}
