use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tracing::{debug, instrument, warn};

use crate::config::McpServerConfig;
use crate::logging::SharedEventSink;
use crate::mcp::{Error, ToolExecutor};

pub struct McpRegistry {
    servers: HashMap<String, ServerHandle>,
    logger: Option<SharedEventSink>,
}

struct ServerHandle {
    tools: HashSet<String>,
    client: McpClientHandle,
}

struct McpClientHandle {
    server_name: String,
    tx: mpsc::Sender<ActorRequest>,
}

struct ActorRequest {
    method: String,
    params: Value,
    reply: oneshot::Sender<Result<Value, Error>>,
}

struct McpStdioClient {
    server_name: String,
    timeout: Duration,
    #[allow(dead_code)]
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: AtomicU64,
}

impl McpRegistry {
    /// Connects to all configured MCP servers and discovers their tools.
    ///
    /// # Errors
    ///
    /// Returns [`crate::mcp::Error`] if a server fails to start, initialize, or list tools.
    pub async fn connect_all(
        configs: &[McpServerConfig],
        logger: Option<SharedEventSink>,
    ) -> Result<Self, Error> {
        let mut servers = HashMap::with_capacity(configs.len());

        for cfg in configs {
            let mut client = McpStdioClient::connect(cfg)?;
            client.initialize().await?;
            let tools = client.list_tools().await?;
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
                .map_err(|err| Error::Protocol {
                    server: cfg.name.clone(),
                    error: err.to_string(),
                })?;
            }

            let handle = spawn_actor(cfg.name.clone(), client);

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
}

fn spawn_actor(server_name: String, client: McpStdioClient) -> McpClientHandle {
    let (tx, mut rx) = mpsc::channel::<ActorRequest>(32);
    let actor_name = server_name.clone();
    tokio::spawn(async move {
        let mut client = client;
        while let Some(req) = rx.recv().await {
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
    });
    McpClientHandle { server_name, tx }
}

/// Continuously drains MCP stderr so a verbose server cannot fill the pipe and deadlock.
fn spawn_stderr_drain(server_name: String, stderr: ChildStderr) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        debug!(server = %server_name, stderr = %trimmed, "mcp server stderr");
                    }
                }
                Err(error) => {
                    warn!(
                        server = %server_name,
                        error = %error,
                        "mcp stderr drain stopped"
                    );
                    break;
                }
            }
        }
    });
}

impl McpClientHandle {
    async fn request(&self, method: &str, params: Value) -> Result<Value, Error> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ActorRequest {
                method: method.to_string(),
                params,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::ActorClosed {
                server: self.server_name.clone(),
            })?;
        reply_rx.await.map_err(|_| Error::ActorClosed {
            server: self.server_name.clone(),
        })?
    }
}

#[async_trait]
impl ToolExecutor for McpRegistry {
    #[instrument(skip(self, arguments), fields(server, tool))]
    async fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value, Error> {
        tracing::Span::current().record("server", server);
        tracing::Span::current().record("tool", tool);

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

        let arguments_for_log = self.logger.as_ref().map(|_| arguments.clone());
        let result = handle
            .client
            .request(
                "tools/call",
                json!({
                    "name": tool,
                    "arguments": arguments,
                }),
            )
            .await?;

        if let Some(log) = &self.logger {
            log.write_event(
                "mcp_tool_call",
                json!({
                    "server": server,
                    "tool": tool,
                    "arguments": arguments_for_log,
                    "result": result,
                }),
            )
            .await
            .map_err(|err| Error::Protocol {
                server: server.to_string(),
                error: err.to_string(),
            })?;
        }

        Ok(result)
    }

    fn available_tools(&self) -> Vec<String> {
        let mut entries = Vec::new();
        for (server, handle) in &self.servers {
            for tool in &handle.tools {
                entries.push(format!("{server}.{tool}"));
            }
        }
        entries.sort();
        entries
    }
}

impl McpStdioClient {
    fn connect(cfg: &McpServerConfig) -> Result<Self, Error> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|source| Error::Spawn {
            server: cfg.name.clone(),
            source,
        })?;
        let stdin = child.stdin.take().ok_or_else(|| Error::MissingPipe {
            server: cfg.name.clone(),
            pipe: "stdin".to_string(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| Error::MissingPipe {
            server: cfg.name.clone(),
            pipe: "stdout".to_string(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| Error::MissingPipe {
            server: cfg.name.clone(),
            pipe: "stderr".to_string(),
        })?;
        spawn_stderr_drain(cfg.name.clone(), stderr);

        Ok(Self {
            server_name: cfg.name.clone(),
            timeout: Duration::from_millis(cfg.timeout_ms),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: AtomicU64::new(1),
        })
    }

    async fn initialize(&mut self) -> Result<(), Error> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "agent_Kuibyshev",
                "version": "0.1.0"
            }
        });
        let _ = self.request("initialize", params).await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<String>, Error> {
        let response = self.request("tools/list", json!({})).await?;
        let list = response
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(list.len());
        for entry in list {
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                out.push(name.to_string());
            }
        }
        Ok(out)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), Error> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_frame(&payload).await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_frame(&payload).await?;

        let server = self.server_name.clone();
        let method_name = method.to_string();
        let timeout_duration = self.timeout;
        let fut = self.wait_for_response(id);
        timeout(timeout_duration, fut)
            .await
            .map_err(|_| Error::Timeout {
                server,
                method: method_name,
            })?
    }

    async fn wait_for_response(&mut self, target_id: u64) -> Result<Value, Error> {
        loop {
            let message = self.read_frame().await?;
            let Some(id_value) = message.get("id") else {
                continue;
            };
            if id_value.as_u64() != Some(target_id) {
                continue;
            }

            if let Some(err) = message.get("error") {
                return Err(Error::Protocol {
                    server: self.server_name.clone(),
                    error: err.to_string(),
                });
            }

            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn write_frame(&mut self, payload: &Value) -> Result<(), Error> {
        let body = serde_json::to_vec(payload)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_frame(&mut self) -> Result<Value, Error> {
        let mut content_length = None::<usize>;
        loop {
            let mut line = String::new();
            let bytes = self.stdout.read_line(&mut line).await?;
            if bytes == 0 {
                return Err(Error::Protocol {
                    server: self.server_name.clone(),
                    error: "unexpected EOF from MCP server".to_string(),
                });
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("content-length:") {
                let parsed = rest
                    .trim()
                    .parse::<usize>()
                    .map_err(|err| Error::Protocol {
                        server: self.server_name.clone(),
                        error: format!("invalid content-length header: {err}"),
                    })?;
                content_length = Some(parsed);
            }
        }
        let size = content_length.ok_or_else(|| Error::Protocol {
            server: self.server_name.clone(),
            error: "missing content-length header".to_string(),
        })?;
        let mut body = vec![0u8; size];
        self.stdout.read_exact(&mut body).await?;
        Ok(serde_json::from_slice(&body)?)
    }
}
