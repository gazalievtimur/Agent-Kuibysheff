//! ACP stdio agent server backed by Kuibysheff `AgentEngine`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, Implementation, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    SessionId, SessionNotification, StopReason as AcpStopReason,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error as AcpError, Responder, Stdio};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

use crate::acp::map::{map_agent_event, map_stop_reason};
use crate::agent::{AgentEvent, AgentEventTx, RunCancel};
use crate::app::{run_agent_prompt, AgentPromptArgs};
use crate::cli::AcpArgs;
use crate::output::StopReason;
use crate::project_paths::{effective_project_root, resolve_agent_paths};

struct SessionSlot {
    cancel: RunCancel,
    cwd: PathBuf,
    touched_seq: u64,
}

struct AcpRuntime {
    args: AcpArgs,
    sessions: Mutex<HashMap<String, SessionSlot>>,
    session_seq: AtomicU64,
    session_touch_seq: AtomicU64,
}

const MAX_ACTIVE_SESSIONS: usize = 256;

impl AcpRuntime {
    fn new(args: AcpArgs) -> Self {
        Self {
            args,
            sessions: Mutex::new(HashMap::new()),
            session_seq: AtomicU64::new(1),
            session_touch_seq: AtomicU64::new(1),
        }
    }

    fn next_session_id(&self) -> SessionId {
        let n = self.session_seq.fetch_add(1, Ordering::Relaxed);
        SessionId::new(format!("kuib-{n}"))
    }

    fn next_touch_seq(&self) -> u64 {
        self.session_touch_seq.fetch_add(1, Ordering::Relaxed)
    }
}

/// Serve ACP over stdio until the client disconnects.
///
/// # Errors
///
/// Returns an ACP error when the JSON-RPC transport fails.
pub async fn run_acp_server(args: AcpArgs) -> Result<(), AcpError> {
    let runtime = Arc::new(AcpRuntime::new(args));

    info!(
        config = %runtime.args.config.display(),
        home = %runtime.args.home.display(),
        project_root = ?runtime.args.project_root.as_ref().map(|p| p.display().to_string()),
        "starting ACP agent on stdio"
    );

    let rt_session = Arc::clone(&runtime);
    let rt_prompt = Arc::clone(&runtime);
    let rt_cancel = Arc::clone(&runtime);

    Agent
        .builder()
        .name("agent_Kuibysheff")
        .on_receive_request(
            async move |req: InitializeRequest, responder, _cx| {
                let version = env!("CARGO_PKG_VERSION");
                responder.respond(
                    InitializeResponse::new(req.protocol_version)
                        .agent_capabilities(AgentCapabilities::new())
                        .agent_info(
                            Implementation::new("agent_Kuibysheff", version)
                                .title("agent_Kuibysheff"),
                        ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                let session_id = rt_session.next_session_id();
                let id_key = session_id.0.to_string();
                let cwd = req.cwd;
                info!(session_id = %id_key, cwd = %cwd.display(), "ACP session created");
                let mut sessions = rt_session.sessions.lock().await;
                sessions.insert(
                    id_key.clone(),
                    SessionSlot {
                        cancel: RunCancel::new(),
                        cwd,
                        touched_seq: rt_session.next_touch_seq(),
                    },
                );
                if sessions.len() > MAX_ACTIVE_SESSIONS {
                    let evicted = sessions
                        .iter()
                        .min_by_key(|(_, slot)| slot.touched_seq)
                        .map(|(key, _)| key.clone())
                        .and_then(|oldest| {
                            if oldest != id_key {
                                sessions.remove(&oldest)?;
                                Some(oldest)
                            } else {
                                None
                            }
                        });
                    if let Some(evicted_id) = evicted {
                        warn!(
                            session_id = %evicted_id,
                            max_sessions = MAX_ACTIVE_SESSIONS,
                            "ACP evicted least-recently-used session"
                        );
                    }
                }
                responder.respond(NewSessionResponse::new(session_id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: PromptRequest, responder, cx| {
                let runtime = Arc::clone(&rt_prompt);
                let connection = cx.clone();
                cx.spawn(async move { handle_prompt(runtime, req, responder, connection).await })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notif: CancelNotification, _cx| {
                let id_key = notif.session_id.0.to_string();
                if let Some(slot) = rt_cancel.sessions.lock().await.get(&id_key) {
                    info!(session_id = %id_key, "ACP session/cancel");
                    slot.cancel.cancel();
                } else {
                    warn!(session_id = %id_key, "ACP cancel for unknown session");
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
}

async fn handle_prompt(
    runtime: Arc<AcpRuntime>,
    req: PromptRequest,
    responder: Responder<PromptResponse>,
    connection: ConnectionTo<Client>,
) -> Result<(), AcpError> {
    let session_id = req.session_id.clone();
    let id_key = session_id.0.to_string();

    let (cancel, session_cwd) = {
        let mut sessions = runtime.sessions.lock().await;
        let Some(slot) = sessions.get_mut(&id_key) else {
            return responder.respond_with_error(AcpError::invalid_params().data(
                serde_json::Value::String(format!("unknown session `{id_key}`")),
            ));
        };
        // Fresh cancel token per prompt (previous turn may have cancelled/expired).
        slot.cancel = RunCancel::new();
        slot.touched_seq = runtime.next_touch_seq();
        (slot.cancel.clone(), slot.cwd.clone())
    };

    let prompt = extract_prompt_text(&req.prompt);
    if prompt.trim().is_empty() {
        return responder.respond_with_error(AcpError::invalid_params().data(
            serde_json::Value::String(
                "prompt must contain at least one text content block".to_string(),
            ),
        ));
    }

    let project_root = effective_project_root(
        Some(session_cwd.as_path()),
        runtime.args.project_root.as_deref(),
    );
    let (config, settings_dir, home) = resolve_agent_paths(
        project_root.as_deref(),
        &runtime.args.config,
        &runtime.args.settings_dir,
        &runtime.args.home,
    );

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let events = AgentEventTx::from_sender(event_tx);
    let forward_conn = connection.clone();
    let forward_sid = session_id.clone();
    let forward = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let update = map_agent_event(event);
            if let Err(err) = forward_conn
                .send_notification(SessionNotification::new(forward_sid.clone(), update))
            {
                warn!(error = %err, "failed to send ACP session/update");
                break;
            }
        }
    });

    let prompt_args = AgentPromptArgs {
        config,
        settings_dir,
        home,
        prompt,
        run_id: None,
        files: Vec::new(),
        max_iterations: runtime.args.max_iterations,
        max_tokens: runtime.args.max_tokens,
        max_duration_sec: runtime.args.max_duration_sec,
        max_cost: runtime.args.max_cost.clone(),
        save_chat_history: runtime.args.save_chat_history,
        cancel: cancel.clone(),
        events,
    };

    let run_result = run_agent_prompt(prompt_args).await;
    // Event sender drops with `prompt_args` / engine request; wait for forwarder to drain.
    let _ = forward.await;

    let cancelled = cancel.is_cancelled();
    match run_result {
        Ok(output) => {
            let stop = map_stop_reason(output.stop_reason.clone(), cancelled);
            if output.stop_reason == StopReason::Error && !cancelled {
                warn!(result = %output.result, "ACP prompt finished with error stop_reason");
            }
            responder.respond(PromptResponse::new(stop))
        }
        Err(err) => {
            error!(error = %err, "ACP prompt wiring failed");
            let _ = connection.send_notification(SessionNotification::new(
                session_id,
                map_agent_event(AgentEvent::Message(format!("{err:#}"))),
            ));
            let stop = if cancelled {
                AcpStopReason::Cancelled
            } else {
                AcpStopReason::Refusal
            };
            responder.respond(PromptResponse::new(stop))
        }
    }
}

fn extract_prompt_text(blocks: &[ContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in blocks {
        if let ContentBlock::Text(text) = block {
            parts.push(text.text.clone());
        }
    }
    parts.join("\n")
}
