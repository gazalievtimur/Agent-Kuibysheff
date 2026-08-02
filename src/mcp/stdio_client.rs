use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tracing::{debug, instrument, warn};

use crate::config::{McpServerConfig, McpStdioConfig, McpTransport};
use crate::logging::SharedEventSink;
use crate::mcp::http_client::McpHttpClient;
use crate::mcp::{Error, ToolExecutor};
use crate::tools::ToolError;

/// Maximum NDJSON frame size (JSON payload + trailing newline), matching SSE buffer default.
const MAX_STDIO_FRAME_BYTES: usize = 1024 * 1024;

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
        if let Self::Http(client) = self {
            if let Err(err) = client.shutdown().await {
                warn!(error = %err, "MCP HTTP session shutdown failed");
            }
        }
    }
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
            let mut client = match &cfg.transport {
                McpTransport::Stdio(stdio) => {
                    let mut client = McpStdioClient::connect(&cfg.name, stdio, cfg.timeout_ms)?;
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

fn spawn_actor(server_name: String, client: LiveClient) -> McpClientHandle {
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
        client.shutdown().await;
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
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        tracing::Span::current().record("server", server);
        tracing::Span::current().record("tool", tool);

        let handle = self
            .servers
            .get(server)
            .ok_or_else(|| ToolError::Mcp(Error::UnknownServer(server.to_string())))?;
        if !handle.tools.contains(tool) {
            return Err(ToolError::Mcp(Error::UnknownTool {
                server: server.to_string(),
                tool: tool.to_string(),
            }));
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
            .map_err(|err| {
                ToolError::Mcp(Error::Logging {
                    server: server.to_string(),
                    source: err,
                })
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
    fn connect(server_name: &str, cfg: &McpStdioConfig, timeout_ms: u64) -> Result<Self, Error> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|source| Error::Spawn {
            server: server_name.to_string(),
            source,
        })?;
        let stdin = child.stdin.take().ok_or_else(|| Error::MissingPipe {
            server: server_name.to_string(),
            pipe: "stdin".to_string(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| Error::MissingPipe {
            server: server_name.to_string(),
            pipe: "stdout".to_string(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| Error::MissingPipe {
            server: server_name.to_string(),
            pipe: "stderr".to_string(),
        })?;
        spawn_stderr_drain(server_name.to_string(), stderr);

        Ok(Self {
            server_name: server_name.to_string(),
            timeout: Duration::from_millis(timeout_ms),
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
        let frame = encode_ndjson_frame(payload, &self.server_name)?;
        self.stdin.write_all(&frame).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_frame(&mut self) -> Result<Value, Error> {
        let line = read_ndjson_line_bytes(&mut self.stdout, &self.server_name).await?;
        decode_ndjson_line(&line, &self.server_name)
    }
}

/// Encode a JSON-RPC message as one NDJSON line (`json\n`).
fn encode_ndjson_frame(payload: &Value, server: &str) -> Result<Vec<u8>, Error> {
    let body = serde_json::to_vec(payload)?;
    if body.len().saturating_add(1) > MAX_STDIO_FRAME_BYTES {
        return Err(Error::Protocol {
            server: server.to_string(),
            error: "MCP stdio frame exceeds max size".to_string(),
        });
    }
    let mut out = body;
    out.push(b'\n');
    Ok(out)
}

/// Parse one NDJSON line (optional trailing `\r` / `\n`).
fn decode_ndjson_line(line: &[u8], server: &str) -> Result<Value, Error> {
    let line = strip_trailing_line_ending(line);
    if looks_like_content_length_framing(line) {
        return Err(Error::Protocol {
            server: server.to_string(),
            error: "expected NDJSON, got Content-Length framing".to_string(),
        });
    }
    Ok(serde_json::from_slice(line)?)
}

fn strip_trailing_line_ending(mut line: &[u8]) -> &[u8] {
    if line.last() == Some(&b'\n') {
        line = &line[..line.len() - 1];
    }
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    line
}

fn looks_like_content_length_framing(line: &[u8]) -> bool {
    const PREFIX: &[u8] = b"content-length:";
    let trimmed = trim_ascii_whitespace_start(line);
    trimmed.len() >= PREFIX.len() && trimmed[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

fn trim_ascii_whitespace_start(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[start..]
}

/// Read one newline-delimited frame with a hard size cap (DoS guard).
async fn read_ndjson_line_bytes<R>(reader: &mut R, server: &str) -> Result<Vec<u8>, Error>
where
    R: AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() {
                return Err(Error::Protocol {
                    server: server.to_string(),
                    error: "unexpected EOF from MCP server".to_string(),
                });
            }
            return Ok(buf);
        }

        if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
            let end = pos + 1;
            if buf.len().saturating_add(end) > MAX_STDIO_FRAME_BYTES {
                return Err(Error::Protocol {
                    server: server.to_string(),
                    error: "MCP stdio frame exceeds max size".to_string(),
                });
            }
            buf.extend_from_slice(&chunk[..end]);
            reader.consume(end);
            return Ok(buf);
        }

        if buf.len().saturating_add(chunk.len()) > MAX_STDIO_FRAME_BYTES {
            return Err(Error::Protocol {
                server: server.to_string(),
                error: "MCP stdio frame exceeds max size".to_string(),
            });
        }
        let n = chunk.len();
        buf.extend_from_slice(chunk);
        reader.consume(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[test]
    fn encode_ndjson_produces_single_trailing_newline_without_headers() {
        let payload = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        let frame = encode_ndjson_frame(&payload, "test").expect("encode");
        assert!(frame.ends_with(b"\n"));
        assert_eq!(frame.iter().filter(|&&b| b == b'\n').count(), 1);
        let text = std::str::from_utf8(&frame).expect("utf8");
        assert!(!text.to_ascii_lowercase().contains("content-length"));
        let parsed: Value = serde_json::from_slice(strip_trailing_line_ending(&frame)).expect("json");
        assert_eq!(parsed, payload);
    }

    #[test]
    fn decode_ndjson_accepts_lf_and_crlf() {
        let lf = br#"{"ok":true}
"#;
        let crlf = br#"{"ok":true}
"#;
        assert_eq!(
            decode_ndjson_line(lf, "test").expect("lf"),
            json!({"ok": true})
        );
        assert_eq!(
            decode_ndjson_line(crlf, "test").expect("crlf"),
            json!({"ok": true})
        );
    }

    #[test]
    fn decode_rejects_content_length_framing() {
        let line = b"Content-Length: 12\r\n";
        let err = decode_ndjson_line(line, "srv").expect_err("content-length");
        match err {
            Error::Protocol { server, error } => {
                assert_eq!(server, "srv");
                assert!(error.contains("expected NDJSON, got Content-Length framing"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn encode_rejects_oversized_frame() {
        // Compact JSON of a large string exceeds the frame budget.
        let huge = "x".repeat(MAX_STDIO_FRAME_BYTES);
        let payload = json!({ "data": huge });
        let err = encode_ndjson_frame(&payload, "srv").expect_err("oversize");
        match err {
            Error::Protocol { error, .. } => {
                assert!(error.contains("MCP stdio frame exceeds max size"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_rejects_oversized_line() {
        let mut oversized = vec![b'a'; MAX_STDIO_FRAME_BYTES];
        oversized.push(b'\n');
        let mut reader = BufReader::new(oversized.as_slice());
        let err = read_ndjson_line_bytes(&mut reader, "srv")
            .await
            .expect_err("oversize");
        match err {
            Error::Protocol { error, .. } => {
                assert!(error.contains("MCP stdio frame exceeds max size"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_and_decode_roundtrip_line() {
        let data = br#"{"jsonrpc":"2.0","id":1,"result":{}}
"#;
        let mut reader = BufReader::new(data.as_slice());
        let line = read_ndjson_line_bytes(&mut reader, "test")
            .await
            .expect("read");
        let value = decode_ndjson_line(&line, "test").expect("decode");
        assert_eq!(value["id"], 1);
    }
}
