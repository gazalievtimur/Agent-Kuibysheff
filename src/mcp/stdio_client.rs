use std::collections::{HashMap, HashSet};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, instrument, warn};

use crate::config::{McpServerConfig, McpStdioConfig, McpTransport};
use crate::logging::SharedEventSink;
use crate::mcp::http_client::McpHttpClient;
use crate::mcp::{Error, ToolExecutor};
use crate::tools::ToolError;

/// Maximum NDJSON frame size (JSON payload + trailing newline), matching SSE buffer default.
const MAX_STDIO_FRAME_BYTES: usize = 1024 * 1024;

/// Extra time allowed for the actor to finish after the child grace period.
const ACTOR_SHUTDOWN_SLACK: Duration = Duration::from_secs(2);

/// Connected MCP servers and their discovered tools.
///
/// Prefer [`McpRegistry::shutdown`] over dropping: drop only closes actor channels
/// (best-effort). Stdio children then rely on actor shutdown or `kill_on_drop`.
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
    tx: Option<mpsc::Sender<ActorRequest>>,
    join: Option<JoinHandle<()>>,
    /// Grace period used when awaiting the actor after the request channel closes.
    shutdown_timeout: Duration,
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

struct McpStdioClient {
    server_name: String,
    timeout: Duration,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
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

            let shutdown_timeout = match &client {
                LiveClient::Stdio(c) => c.timeout,
                LiveClient::Http(_) => Duration::from_secs(30),
            };
            let handle = spawn_actor(cfg.name.clone(), client, shutdown_timeout);

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
}

fn spawn_actor(
    server_name: String,
    client: LiveClient,
    shutdown_timeout: Duration,
) -> McpClientHandle {
    let (tx, mut rx) = mpsc::channel::<ActorRequest>(32);
    let actor_name = server_name.clone();
    let join = tokio::spawn(async move {
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
    McpClientHandle {
        server_name,
        tx: Some(tx),
        join: Some(join),
        shutdown_timeout,
    }
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
        let tx = self.tx.as_ref().ok_or_else(|| Error::ActorClosed {
            server: self.server_name.clone(),
        })?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(ActorRequest {
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
        // Safety net when Drop runs without an awaited shutdown (see Drop impl).
        cmd.kill_on_drop(true);
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
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(BufReader::new(stdout)),
            next_id: AtomicU64::new(1),
        })
    }

    /// Close stdin, wait for exit (with kill fallback), and log the status.
    async fn shutdown(&mut self) {
        // Close pipes so a well-behaved server can exit on EOF.
        self.stdin.take();
        self.stdout.take();
        let Some(child) = self.child.take() else {
            return;
        };
        shutdown_stdio_child(&self.server_name, child, self.timeout).await;
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
        let stdin = self.stdin.as_mut().ok_or_else(|| Error::Protocol {
            server: self.server_name.clone(),
            error: "stdio client already shut down".to_string(),
        })?;
        stdin.write_all(&frame).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_frame(&mut self) -> Result<Value, Error> {
        let stdout = self.stdout.as_mut().ok_or_else(|| Error::Protocol {
            server: self.server_name.clone(),
            error: "stdio client already shut down".to_string(),
        })?;
        let line = read_ndjson_line_bytes(stdout, &self.server_name).await?;
        decode_ndjson_line(&line, &self.server_name)
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        // Cannot await wait()/kill() in Drop. Close stdin and rely on kill_on_drop(true).
        warn!(
            server = %self.server_name,
            "MCP stdio client dropped without shutdown; child termination is best-effort"
        );
        self.stdin.take();
        self.stdout.take();
    }
}

/// Close-stdin → wait(timeout) → kill → wait. Logs exit code/signal (never env).
async fn shutdown_stdio_child(server_name: &str, mut child: Child, grace: Duration) {
    match timeout(grace, child.wait()).await {
        Ok(Ok(status)) => {
            log_child_exit(server_name, status, false);
            return;
        }
        Ok(Err(error)) => {
            warn!(
                server = %server_name,
                error = %error,
                "MCP stdio child wait failed"
            );
            return;
        }
        Err(_) => {
            warn!(
                server = %server_name,
                grace_ms = grace.as_millis() as u64,
                "MCP stdio child did not exit after stdin close; killing"
            );
        }
    }

    if let Err(error) = child.start_kill() {
        warn!(
            server = %server_name,
            error = %error,
            "MCP stdio child kill failed"
        );
        return;
    }

    match timeout(grace, child.wait()).await {
        Ok(Ok(status)) => log_child_exit(server_name, status, true),
        Ok(Err(error)) => {
            warn!(
                server = %server_name,
                error = %error,
                "MCP stdio child wait after kill failed"
            );
        }
        Err(_) => {
            warn!(
                server = %server_name,
                "MCP stdio child did not exit after kill"
            );
        }
    }
}

fn log_child_exit(server_name: &str, status: ExitStatus, killed: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            debug!(
                server = %server_name,
                signal,
                killed,
                "MCP stdio child exited by signal"
            );
            return;
        }
    }
    debug!(
        server = %server_name,
        exit_code = ?status.code(),
        killed,
        "MCP stdio child exited"
    );
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

    #[tokio::test]
    async fn shutdown_stdio_child_exits_after_stdin_close() {
        let mut child = hang_or_echo_command(false)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn");
        drop(child.stdin.take());
        drop(child.stdout.take());
        let pid = child.id().expect("pid");
        shutdown_stdio_child("graceful", child, Duration::from_secs(5)).await;
        assert!(
            !process_alive(pid),
            "pid {pid} should be gone after stdin-close shutdown"
        );
    }

    #[tokio::test]
    async fn shutdown_stdio_child_kills_hung_process() {
        let mut child = hang_or_echo_command(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn hang");
        drop(child.stdin.take());
        let pid = child.id().expect("pid");
        shutdown_stdio_child("hang", child, Duration::from_millis(200)).await;
        assert!(
            !process_alive(pid),
            "hung pid {pid} should be gone after kill"
        );
    }

    /// `hang == false`: process exits when stdin closes.
    /// `hang == true`: process ignores stdin and runs until killed.
    fn hang_or_echo_command(hang: bool) -> Command {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("powershell");
            if hang {
                cmd.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
            } else {
                // Drain stdin until EOF, then exit.
                cmd.args([
                    "-NoProfile",
                    "-Command",
                    "$in = [Console]::OpenStandardInput(); $buf = New-Object byte[] 4096; while (($n = $in.Read($buf,0,4096)) -gt 0) {}",
                ]);
            }
            cmd
        }
        #[cfg(unix)]
        {
            if hang {
                let mut cmd = Command::new("sleep");
                cmd.arg("60");
                cmd
            } else {
                Command::new("cat")
            }
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = hang;
            panic!("unsupported platform for shutdown test helper");
        }
    }

    fn process_alive(pid: u32) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .output();
            match output {
                Ok(out) => {
                    let text = String::from_utf8_lossy(&out.stdout);
                    text.contains(&pid.to_string())
                }
                Err(_) => false,
            }
        }
        #[cfg(unix)]
        {
            let status = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status();
            matches!(status, Ok(s) if s.success())
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = pid;
            false
        }
    }
}
