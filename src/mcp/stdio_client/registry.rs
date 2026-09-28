//! Connected MCP servers: actor loop, tool discovery, and shutdown.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::process::{ensure_stdio_sandbox_available, McpStdioClient};
use crate::config::{McpServerConfig, McpTransport};
use crate::logging::SharedEventSink;
use crate::mcp::http_client::McpHttpClient;
use crate::mcp::Error;

/// Extra time allowed for the actor to finish after the child grace period.
const ACTOR_SHUTDOWN_SLACK: Duration = Duration::from_secs(2);

/// Isolation inputs for MCP stdio children (protected-store boundary).
#[derive(Debug, Clone, Default)]
pub struct McpIsolationContext {
    pub project_root: Option<PathBuf>,
    pub agent_id: String,
}

/// Connected MCP servers and their discovered tools.
///
/// Prefer [`McpRegistry::shutdown`] over dropping: drop only closes actor channels
/// (best-effort). Stdio children then rely on actor shutdown or `kill_on_drop`.
pub struct McpRegistry {
    pub(super) servers: HashMap<String, ServerHandle>,
    pub(super) logger: Option<SharedEventSink>,
}

pub(super) struct ServerHandle {
    pub(super) tools: HashSet<String>,
    client: McpClientHandle,
}

struct McpClientHandle {
    server_name: String,
    tx: Option<mpsc::Sender<ActorRequest>>,
    join: Option<JoinHandle<()>>,
    /// Grace period used when awaiting the actor after the request channel closes.
    shutdown_timeout: Duration,
    cancel: CancellationToken,
}

struct ActorRequest {
    method: String,
    params: Value,
    reply: oneshot::Sender<Result<Value, Error>>,
}

enum LiveClient {
    Stdio(Box<McpStdioClient>),
    Http(Box<McpHttpClient>),
}

impl LiveClient {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        match self {
            Self::Stdio(client) => client.request(method, params).await,
            Self::Http(client) => client.request(method, params).await,
        }
    }

    async fn shutdown(&mut self) {
        match self {
            Self::Stdio(client) => client.shutdown().await,
            Self::Http(client) => {
                if let Err(err) = client.shutdown().await {
                    warn!(error = %err, "MCP HTTP session shutdown failed");
                }
            }
        }
    }
}

impl McpRegistry {
    /// Connects to all configured MCP servers and discovers their tools.
    ///
    /// Relative stdio `command` / `args` resolve against each server's optional
    /// `cwd`, otherwise `config_dir`, otherwise the process CWD.
    ///
    /// # Errors
    ///
    /// Returns [`crate::mcp::Error`] if a server fails to start, initialize, or list tools.
    pub async fn connect_all(
        configs: &[McpServerConfig],
        logger: Option<SharedEventSink>,
        cancel: CancellationToken,
    ) -> Result<Self, Error> {
        Self::connect_all_isolated(configs, logger, cancel, McpIsolationContext::default()).await
    }

    /// Like [`Self::connect_all`], with an explicit config-file directory for
    /// resolving relative stdio MCP paths (legacy tests).
    ///
    /// # Errors
    ///
    /// Returns [`crate::mcp::Error`] if a server fails to start, initialize, or list tools.
    pub async fn connect_all_with_config_dir(
        configs: &[McpServerConfig],
        logger: Option<SharedEventSink>,
        cancel: CancellationToken,
        config_dir: Option<&Path>,
    ) -> Result<Self, Error> {
        let mut isolation = McpIsolationContext::default();
        if let Some(dir) = config_dir {
            isolation.project_root = dir.parent().map(Path::to_path_buf);
        }
        Self::connect_all_isolated(configs, logger, cancel, isolation).await
    }

    /// Connect MCP servers with protected-store isolation for stdio children.
    ///
    /// Stdio MCP requires a working OS sandbox (`SandboxRunner::probe`) unless
    /// `KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1` is set (dev/emergency only). Child cwd is
    /// `mcp-runtime/{agent}/{server}/` under the project `.kuibysheff` tree; env is cleared.
    ///
    /// # Errors
    ///
    /// Returns [`crate::mcp::Error`] if a server fails isolation, start, initialize, or list tools.
    pub async fn connect_all_isolated(
        configs: &[McpServerConfig],
        logger: Option<SharedEventSink>,
        cancel: CancellationToken,
        isolation: McpIsolationContext,
    ) -> Result<Self, Error> {
        let needs_stdio = configs
            .iter()
            .any(|c| matches!(c.transport, McpTransport::Stdio(_)));
        if needs_stdio {
            ensure_stdio_sandbox_available(&isolation)?;
        }

        let mut servers = HashMap::with_capacity(configs.len());

        for cfg in configs {
            let mut client = match &cfg.transport {
                McpTransport::Stdio(stdio) => {
                    let mut client = McpStdioClient::connect_isolated(
                        &cfg.name,
                        stdio,
                        cfg.timeout_ms,
                        &isolation,
                    )
                    .await?;
                    client.initialize().await?;
                    LiveClient::Stdio(Box::new(client))
                }
                McpTransport::Http(http) => {
                    let client = McpHttpClient::connect(&cfg.name, http, cfg.timeout_ms).await?;
                    LiveClient::Http(Box::new(client))
                }
            };

            let tools = match &mut client {
                LiveClient::Stdio(c) => c.list_tools().await?,
                LiveClient::Http(c) => c.list_tools().await?,
            };
            let tool_set: HashSet<_> = tools.into_iter().collect();

            if let Some(log) = &logger {
                log.write_event(
                    "mcp_server_initialized",
                    json!({
                        "server": cfg.name,
                        "tools": tool_set.iter().cloned().collect::<Vec<_>>(),
                    }),
                )
                .await
                .map_err(|err| Error::Logging {
                    server: cfg.name.clone(),
                    source: err,
                })?;
            }

            let shutdown_timeout = match &client {
                LiveClient::Stdio(c) => c.timeout,
                LiveClient::Http(_) => Duration::from_secs(30),
            };
            let handle = spawn_actor(cfg.name.clone(), client, shutdown_timeout, cancel.clone());

            servers.insert(
                cfg.name.clone(),
                ServerHandle {
                    tools: tool_set,
                    client: handle,
                },
            );
        }

        Ok(Self { servers, logger })
    }

    /// Gracefully disconnects all MCP servers and waits for stdio children to exit.
    ///
    /// Prefer this over dropping the registry: [`Drop`] only closes actor channels and is
    /// best-effort (stdio kill may run without awaiting exit).
    pub async fn shutdown(self) {
        for (_, handle) in self.servers {
            handle.client.shutdown().await;
        }
    }

    /// Calls a discovered MCP tool for an Event-MCP pipeline without recording payload bodies.
    pub(crate) async fn call_event_handler(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, Error> {
        self.call_discovered_tool(server, tool, arguments).await
    }

    /// Calls a discovered host-owned billing tool without model-tool payload logging.
    pub(crate) async fn call_billing_handler(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, Error> {
        self.call_discovered_tool(server, tool, arguments).await
    }

    pub(super) async fn call_discovered_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, Error> {
        let handle = self
            .servers
            .get(server)
            .ok_or_else(|| Error::UnknownServer(server.to_string()))?;
        if !handle.tools.contains(tool) {
            return Err(Error::UnknownTool {
                server: server.to_string(),
                tool: tool.to_string(),
            });
        }

        handle
            .client
            .request(
                "tools/call",
                json!({
                    "name": tool,
                    "arguments": arguments,
                }),
            )
            .await
    }

    /// Test helper: registry with one server whose every request returns `result`.
    #[cfg(test)]
    pub(crate) fn with_stub_server(
        server: &str,
        tool: &str,
        result: Value,
        logger: Option<SharedEventSink>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<ActorRequest>(8);
        let join = tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                let _ = req.reply.send(Ok(result.clone()));
            }
        });
        let mut tools = HashSet::new();
        tools.insert(tool.to_string());
        let mut servers = HashMap::new();
        servers.insert(
            server.to_string(),
            ServerHandle {
                tools,
                client: McpClientHandle {
                    server_name: server.to_string(),
                    tx: Some(tx),
                    join: Some(join),
                    shutdown_timeout: Duration::from_secs(1),
                    cancel: CancellationToken::new(),
                },
            },
        );
        Self { servers, logger }
    }
}

fn spawn_actor(
    server_name: String,
    client: LiveClient,
    shutdown_timeout: Duration,
    cancel: CancellationToken,
) -> McpClientHandle {
    let (tx, mut rx) = mpsc::channel::<ActorRequest>(32);
    let actor_name = server_name.clone();
    let actor_cancel = cancel.clone();
    let join = tokio::spawn(async move {
        let mut client = client;
        loop {
            tokio::select! {
                biased;
                () = actor_cancel.cancelled() => {
                    debug!(server = %actor_name, "MCP actor cancelled; shutting down");
                    break;
                }
                maybe_req = rx.recv() => {
                    let Some(req) = maybe_req else {
                        break;
                    };
                    let ActorRequest {
                        method,
                        params,
                        reply,
                    } = req;
                    let result = client.request(&method, params).await;
                    if reply.send(result).is_err() {
                        debug!(server = %actor_name, "MCP actor caller dropped before reply");
                    }
                }
            }
        }
        client.shutdown().await;
    });
    McpClientHandle {
        server_name,
        tx: Some(tx),
        join: Some(join),
        shutdown_timeout,
        cancel,
    }
}

impl McpClientHandle {
    async fn request(&self, method: &str, params: Value) -> Result<Value, Error> {
        if self.cancel.is_cancelled() {
            return Err(Error::Cancelled {
                server: self.server_name.clone(),
            });
        }
        let tx = self.tx.as_ref().ok_or_else(|| Error::ActorClosed {
            server: self.server_name.clone(),
        })?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => {
                return Err(Error::Cancelled {
                    server: self.server_name.clone(),
                });
            }
            send_result = tx.send(ActorRequest {
                method: method.to_string(),
                params,
                reply: reply_tx,
            }) => {
                send_result.map_err(|_| Error::ActorClosed {
                    server: self.server_name.clone(),
                })?;
            }
        }
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(Error::Cancelled {
                server: self.server_name.clone(),
            }),
            reply = reply_rx => reply.map_err(|_| Error::ActorClosed {
                server: self.server_name.clone(),
            })?,
        }
    }

    async fn shutdown(mut self) {
        // Closing the request channel lets the actor run LiveClient::shutdown.
        self.tx.take();
        let Some(join) = self.join.take() else {
            return;
        };
        let join_timeout = self
            .shutdown_timeout
            .saturating_mul(2)
            .saturating_add(ACTOR_SHUTDOWN_SLACK);
        match timeout(join_timeout, join).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                warn!(
                    server = %self.server_name,
                    error = %err,
                    "MCP actor task failed during shutdown"
                );
            }
            Err(_) => {
                warn!(
                    server = %self.server_name,
                    "MCP actor shutdown timed out"
                );
            }
        }
    }
}

impl Drop for McpClientHandle {
    fn drop(&mut self) {
        // Best-effort: closing `tx` wakes the actor so it can shut down the live client.
        // The JoinHandle is detached; prefer [`McpRegistry::shutdown`] to await exit.
        self.tx.take();
    }
}
