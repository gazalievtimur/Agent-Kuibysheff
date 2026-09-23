//! Compose axum routers from `a2a-server-lf` and serve the A2A HTTP endpoint.

use std::net::SocketAddr;
use std::sync::Arc;

use a2a_server::agent_card::agent_card_router;
use a2a_server::{jsonrpc, rest, DefaultRequestHandler, InMemoryTaskStore, StaticAgentCard};
use anyhow::{Context, Result};
use axum::middleware;
use axum::Router;
use tokio::signal;
use tracing::info;

use crate::a2a::auth::{require_bearer, BearerToken};
use crate::a2a::card::{
    build_static_card, default_agent_capabilities, resolve_public_url, CardOptions,
};
use crate::a2a::executor::{ExecutorConfig, KuibysheffExecutor, TaskRunner};
use crate::cli::A2aArgs;

/// Run the A2A HTTP server until the process is stopped.
///
/// # Errors
///
/// Returns when bind fails, the token env is invalid, or the profile cannot be loaded.
pub async fn run_a2a_server(args: A2aArgs) -> Result<()> {
    let bind: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid --bind address `{}`", args.bind))?;

    let bearer = match args.token_env.as_deref() {
        Some(var) => Some(BearerToken::from_env(var)?),
        None => None,
    };

    let public_url = resolve_public_url(&args.bind, args.public_url.as_deref());

    let config = ExecutorConfig::from_a2a_args(&args)?;

    let card_producer = Arc::new(build_static_card(&CardOptions {
        agent_id: config.paths.agent_id.clone(),
        settings_dir: config.paths.settings_dir.clone(),
        public_url: public_url.clone(),
        require_bearer: bearer.is_some(),
    })?);

    let executor = KuibysheffExecutor::new(config);
    let cancels = executor.cancels();
    let tasks = executor.tasks();
    let app = build_router(executor, card_producer, bearer);

    info!(
        agent = %args.identity.agent,
        %bind,
        %public_url,
        "starting A2A server"
    );
    info!(
        card = %format!("{public_url}/.well-known/agent-card.json"),
        jsonrpc = %format!("{public_url}/jsonrpc"),
        rest = %format!("{public_url}/rest"),
        "A2A endpoints"
    );

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind A2A listener on {bind}"))?;

    let shutdown = async move {
        if signal::ctrl_c().await.is_err() {
            return;
        }
        info!("A2A shutdown requested, cancelling in-flight tasks");
        {
            let guard = cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for cancel in guard.values() {
                cancel.cancel();
            }
        }
        {
            tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .abort_all();
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("A2A HTTP server exited with error")?;
    Ok(())
}

/// Build the axum application (card + JSON-RPC + REST).
pub fn build_router<R: TaskRunner>(
    executor: KuibysheffExecutor<R>,
    card_producer: Arc<StaticAgentCard>,
    bearer: Option<BearerToken>,
) -> Router {
    let handler = Arc::new(
        DefaultRequestHandler::new(executor, InMemoryTaskStore::new())
            .with_capabilities(default_agent_capabilities()),
    );

    let mut rpc = Router::new()
        .nest("/jsonrpc", jsonrpc::jsonrpc_router(handler.clone()))
        .nest("/rest", rest::rest_router(handler));

    if let Some(token) = bearer {
        rpc = rpc.layer(middleware::from_fn_with_state(token, require_bearer));
    }

    rpc.merge(agent_card_router(card_producer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::executor::{ExecutorConfig, TaskRunner};
    use crate::app::AgentPromptArgs;
    use crate::output::{RunOutput, StopReason};
    use crate::project_paths::ResolvedAgentPaths;
    use a2a::TRANSPORT_PROTOCOL_JSONRPC;
    use a2a_server::StaticAgentCard;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct FakeRunner;

    impl TaskRunner for FakeRunner {
        async fn run(&self, args: AgentPromptArgs) -> anyhow::Result<RunOutput> {
            Ok(RunOutput {
                run_id: "fake".into(),
                result: format!("echo:{}", args.prompt),
                usage: Default::default(),
                stop_reason: StopReason::GoalReached,
                logs: Default::default(),
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

    fn test_card(public_url: &str) -> StaticAgentCard {
        use a2a::{AgentCard, AgentInterface, AgentSkill};
        StaticAgentCard::new(AgentCard {
            name: "demo".into(),
            description: "test agent".into(),
            version: "0.2.0".into(),
            supported_interfaces: vec![
                AgentInterface::new(format!("{public_url}/jsonrpc"), TRANSPORT_PROTOCOL_JSONRPC),
                AgentInterface::new(
                    format!("{public_url}/rest"),
                    a2a::TRANSPORT_PROTOCOL_HTTP_JSON,
                ),
            ],
            capabilities: default_agent_capabilities(),
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            skills: vec![AgentSkill {
                id: "demo".into(),
                name: "demo".into(),
                description: "demo".into(),
                tags: vec![],
                examples: None,
                input_modes: None,
                output_modes: None,
                security_requirements: None,
            }],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        })
    }

    async fn serve_app(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let base = format!("http://{addr}");
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            ready_tx.send(()).ok();
            let _ = axum::serve(listener, app).await;
        });
        ready_rx.await.expect("server ready");
        (base, handle)
    }

    #[tokio::test]
    async fn agent_card_and_send_message_roundtrip() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let base = format!("http://{addr}");

        let app = build_router(
            KuibysheffExecutor::with_runner(test_config(), FakeRunner),
            Arc::new(test_card(&base)),
            None,
        );
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            ready_tx.send(()).ok();
            let _ = axum::serve(listener, app).await;
        });
        ready_rx.await.expect("server ready");

        // Bypass OS/Docker HTTP proxies so loopback test traffic stays local.
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("http client");
        let card_resp = http
            .get(format!("{base}/.well-known/agent-card.json"))
            .send()
            .await
            .expect("card get");
        assert!(card_resp.status().is_success());
        let card: a2a::AgentCard = card_resp.json().await.expect("card json");
        assert_eq!(card.name, "demo");
        assert!(card
            .supported_interfaces
            .iter()
            .any(|i| i.url.contains("/jsonrpc")));

        // Drive JSON-RPC over the same no-proxy client. a2a-client's default
        // factory uses a separate reqwest 0.13 Client that still honors the
        // OS/Docker system proxy and returns 503 for loopback on this host.
        let send_resp = http
            .post(format!("{base}/jsonrpc"))
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "SendMessage",
                "params": {
                    "message": {
                        "messageId": "m1",
                        "role": "ROLE_USER",
                        "parts": [{"text": "hello"}]
                    }
                }
            }))
            .send()
            .await
            .expect("send");
        assert!(
            send_resp.status().is_success(),
            "send status {}",
            send_resp.status()
        );
        let send_body: serde_json::Value = send_resp.json().await.expect("send json");
        assert!(send_body.get("error").is_none(), "send error: {send_body}");
        let task = send_body
            .pointer("/result/task")
            .or_else(|| send_body.get("result"))
            .unwrap_or_else(|| panic!("task result missing: {send_body}"));
        let task_id = task
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("task id missing: {task}"))
            .to_string();
        let state = task
            .pointer("/status/state")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("task state missing: {task}"));
        assert_eq!(state, "TASK_STATE_COMPLETED");
        let text = task
            .pointer("/status/message/parts/0/text")
            .and_then(|v| v.as_str());
        assert_eq!(text, Some("echo:hello"), "body={send_body}");

        let get_resp = http
            .post(format!("{base}/jsonrpc"))
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "GetTask",
                "params": { "id": task_id }
            }))
            .send()
            .await
            .expect("get");
        assert!(get_resp.status().is_success());
        let get_body: serde_json::Value = get_resp.json().await.expect("get json");
        assert!(get_body.get("error").is_none(), "get error: {get_body}");
        let got_state = get_body
            .pointer("/result/status/state")
            .or_else(|| get_body.pointer("/result/task/status/state"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("get state missing: {get_body}"));
        assert_eq!(got_state, "TASK_STATE_COMPLETED");

        handle.abort();
    }

    #[tokio::test]
    async fn bearer_rejects_unauthenticated_rpc() {
        let token = BearerToken::new("secret-token");
        let app = build_router(
            KuibysheffExecutor::with_runner(test_config(), FakeRunner),
            Arc::new(test_card("http://127.0.0.1:0")),
            Some(token),
        );
        let (base, handle) = serve_app(app).await;

        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("http client");
        // Card stays public.
        let card_status = http
            .get(format!("{base}/.well-known/agent-card.json"))
            .send()
            .await
            .expect("card")
            .status();
        assert!(card_status.is_success());

        // RPC without token → 401.
        let rpc_status = http
            .post(format!("{base}/jsonrpc"))
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "SendMessage",
                "params": {
                    "message": {
                        "messageId": "m1",
                        "role": "ROLE_USER",
                        "parts": [{"text": "hi"}]
                    }
                }
            }))
            .send()
            .await
            .expect("rpc")
            .status();
        assert_eq!(rpc_status, reqwest::StatusCode::UNAUTHORIZED);

        handle.abort();
    }

    #[tokio::test]
    async fn bearer_accepts_authenticated_rpc() {
        let token = BearerToken::new("secret-token");
        let app = build_router(
            KuibysheffExecutor::with_runner(test_config(), FakeRunner),
            Arc::new(test_card("http://127.0.0.1:0")),
            Some(token),
        );
        let (base, handle) = serve_app(app).await;

        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("http client");
        let rpc_status = http
            .post(format!("{base}/jsonrpc"))
            .header("Authorization", "Bearer secret-token")
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "SendMessage",
                "params": {
                    "message": {
                        "messageId": "m1",
                        "role": "ROLE_USER",
                        "parts": [{"text": "hi"}]
                    }
                }
            }))
            .send()
            .await
            .expect("rpc")
            .status();
        assert!(
            rpc_status.is_success(),
            "expected success, got {rpc_status}"
        );

        handle.abort();
    }
}
