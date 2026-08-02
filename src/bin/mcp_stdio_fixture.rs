//! Minimal NDJSON MCP stdio server for integration tests.
//!
//! Modes (first CLI arg):
//! - (default) speak NDJSON and answer `initialize` / `tools/list`
//! - `content-length` reply to the first JSON-RPC request with LSP-style framing

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let content_length_mode = std::env::args().nth(1).as_deref() == Some("content-length");
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
            "tools/list" => json!({
                "tools": [{
                    "name": "echo",
                    "description": "fixture echo tool",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }]
            }),
            _ => {
                write_error(&mut stdout, id, -32601, format!("method not found: {method}"));
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
