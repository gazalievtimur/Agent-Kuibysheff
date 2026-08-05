//! Minimal NDJSON MCP stdio server for integration tests.
//!
//! Modes (first CLI arg):
//! - (default) speak NDJSON and answer `initialize` / `tools/list`
//! - `content-length` reply to the first JSON-RPC request with LSP-style framing
//! - `event` expose an `event_transform` tool that appends its handler id to payload.trace
//! - `hang` ignore stdin and sleep forever (for kill-path tests)
//!
//! Optional env `MCP_FIXTURE_ALIVE_FILE`: create this path on start and remove it on clean exit.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

struct AliveFile(Option<PathBuf>);

impl AliveFile {
    fn from_env() -> Self {
        let path = std::env::var_os("MCP_FIXTURE_ALIVE_FILE").map(PathBuf::from);
        if let Some(ref path) = path {
            let _ = std::fs::write(path, b"alive\n");
        }
        Self(path)
    }
}

impl Drop for AliveFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn main() {
    let _alive = AliveFile::from_env();
    let mode = std::env::args().nth(1);
    match mode.as_deref() {
        Some("hang") => loop {
            thread::sleep(Duration::from_secs(3600));
        },
        Some("content-length") => serve(true, false),
        Some("event") => serve(false, true),
        _ => serve(false, false),
    }
}

fn serve(content_length_mode: bool, event_mode: bool) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut used_content_length = false;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(_) => continue,
        };

        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = msg.get("id").cloned();

        // Notifications have no response.
        if id.is_none() {
            continue;
        }

        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {
                    "name": "mcp_stdio_fixture",
                    "version": "0.1.0"
                }
            }),
            "tools/list" if event_mode => json!({
                "tools": [{
                    "name": "event_transform",
                    "description": "append fixture handler id to payload.trace",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }]
            }),
            "tools/list" => tools_list_result(),
            "tools/call" if event_mode => event_tool_result(&msg),
            _ => {
                write_error(
                    &mut stdout,
                    id,
                    -32601,
                    format!("method not found: {method}"),
                );
                continue;
            }
        };

        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });

        if content_length_mode && !used_content_length {
            used_content_length = true;
            write_content_length(&mut stdout, &response);
        } else {
            write_ndjson(&mut stdout, &response);
        }
    }
}

fn tools_list_result() -> Value {
    json!({
        "tools": [{
            "name": "echo",
            "description": "fixture echo tool",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }]
    })
}

fn event_tool_result(request: &Value) -> Value {
    let handler_id =
        std::env::var("MCP_FIXTURE_HANDLER_ID").unwrap_or_else(|_| "fixture".to_string());
    let mut payload = request
        .pointer("/params/arguments/payload")
        .cloned()
        .unwrap_or(Value::Null);
    let trace = payload.as_object_mut().and_then(|object| {
        object
            .entry("trace")
            .or_insert_with(|| json!([]))
            .as_array_mut()
    });
    if let Some(trace) = trace {
        trace.push(json!(handler_id));
    }
    json!({
        "content": [{
            "type": "text",
            "text": "event transformed"
        }],
        "structuredContent": {
            "action": "replace",
            "payload": payload
        }
    })
}

fn write_ndjson(stdout: &mut impl Write, msg: &Value) {
    let body = serde_json::to_vec(msg).expect("serialize");
    stdout.write_all(&body).expect("write body");
    stdout.write_all(b"\n").expect("write newline");
    stdout.flush().expect("flush");
}

fn write_content_length(stdout: &mut impl Write, msg: &Value) {
    let body = serde_json::to_vec(msg).expect("serialize");
    write!(stdout, "Content-Length: {}\r\n\r\n", body.len()).expect("write header");
    stdout.write_all(&body).expect("write body");
    stdout.flush().expect("flush");
}

fn write_error(stdout: &mut impl Write, id: Option<Value>, code: i64, message: String) {
    write_ndjson(
        stdout,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    );
}
