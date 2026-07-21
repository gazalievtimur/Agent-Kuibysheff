//! Streamable HTTP MCP client (protocol version 2025-11-25).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Client, Method, Response, StatusCode, Url};
use serde_json::{json, Value};
use tokio::time::{timeout, Instant};
use tracing::{debug, warn};

use crate::config::McpHttpConfig;
use crate::mcp::oauth::{
    apply_bearer, parse_www_authenticate, BearerChallenge, McpOAuth,
};
use crate::mcp::sse::{parse_json_data, SseParser};
use crate::mcp::Error;

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const HEADER_SESSION_ID: &str = "mcp-session-id";
const HEADER_PROTOCOL_VERSION: &str = "mcp-protocol-version";
const HEADER_LAST_EVENT_ID: &str = "last-event-id";

/// Streamable HTTP MCP transport client.
pub struct McpHttpClient {
    server_name: String,
    endpoint: Url,
    timeout: Duration,
    http: Client,
    static_headers: HeaderMap,
    session_id: Option<String>,
    protocol_version: String,
    next_id: AtomicU64,
    oauth: Option<McpOAuth>,
    /// Legacy HTTP+SSE POST endpoint discovered via `endpoint` SSE event.
    legacy_message_url: Option<Url>,
}

impl McpHttpClient {
    /// Connects to a Streamable HTTP (or legacy HTTP+SSE) MCP endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the URL/headers are invalid, OAuth setup fails, or initialize fails.
    pub async fn connect(server_name: &str, cfg: &McpHttpConfig, timeout_ms: u64) -> Result<Self, Error> {
        let endpoint = Url::parse(cfg.url.trim()).map_err(|err| Error::Protocol {
            server: server_name.to_string(),
            error: format!("invalid mcp url: {err}"),
        })?;
        let http = Client::builder()
            .timeout(Duration::from_millis(timeout_ms.saturating_mul(4).max(60_000)))
            .no_proxy()
            .build()
            .map_err(|source| Error::Http {
                server: server_name.to_string(),
                source,
            })?;
        let static_headers = build_static_headers(server_name, &cfg.headers)?;
        let oauth = match &cfg.auth {
            Some(auth) => Some(McpOAuth::new(server_name, endpoint.clone(), auth.clone())?),
            None => None,
        };

        let mut client = Self {
            server_name: server_name.to_string(),
            endpoint,
            timeout: Duration::from_millis(timeout_ms),
            http,
            static_headers,
            session_id: None,
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            next_id: AtomicU64::new(1),
            oauth,
            legacy_message_url: None,
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Lists tool names via `tools/list`.
    pub async fn list_tools(&mut self) -> Result<Vec<String>, Error> {
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

    /// Sends a JSON-RPC request and returns the `result` object.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        match self.request_once(method, params.clone()).await {
            Ok(v) => Ok(v),
            Err(Error::SessionExpired { .. }) => {
                debug!(server = %self.server_name, "MCP session expired; re-initializing");
                self.session_id = None;
                self.initialize().await?;
                self.request_once(method, params).await
            }
            Err(Error::Unauthorized { challenge, .. }) => {
                self.handle_unauthorized(challenge.as_ref()).await?;
                self.request_once(method, params).await
            }
            Err(err) => Err(err),
        }
    }

    /// Sends a JSON-RPC notification (expects HTTP 202).
    pub async fn notify(&mut self, method: &str, params: Value) -> Result<(), Error> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let response = self.post_json(&payload).await?;
        let status = response.status();
        if status == StatusCode::ACCEPTED || status.is_success() {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        Err(Error::Protocol {
            server: self.server_name.clone(),
            error: format!("notification `{method}` failed: HTTP {status}: {body}"),
        })
    }

    /// Terminates the MCP session via HTTP DELETE when a session id is present.
    pub async fn shutdown(&mut self) -> Result<(), Error> {
        let Some(session) = self.session_id.clone() else {
            return Ok(());
        };
        let mut req = self
            .http
            .request(Method::DELETE, self.endpoint.clone())
            .header(HEADER_SESSION_ID, session)
            .header(HEADER_PROTOCOL_VERSION, self.protocol_version.as_str());
        req = self.apply_common_headers(req)?;
        let response = req.send().await.map_err(|source| Error::Http {
            server: self.server_name.clone(),
            source,
        })?;
        let status = response.status();
        if status == StatusCode::METHOD_NOT_ALLOWED
            || status == StatusCode::OK
            || status == StatusCode::NO_CONTENT
            || status == StatusCode::ACCEPTED
        {
            self.session_id = None;
            return Ok(());
        }
        warn!(
            server = %self.server_name,
            status = %status,
            "MCP session DELETE returned unexpected status"
        );
        self.session_id = None;
        Ok(())
    }

    async fn initialize(&mut self) -> Result<(), Error> {
        let params = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "agent_Kuibyshev",
                "version": "0.1.0"
            }
        });

        match self.request_once("initialize", params.clone()).await {
            Ok(result) => {
                self.apply_initialize_result(&result)?;
                self.notify("notifications/initialized", json!({})).await?;
                Ok(())
            }
            Err(Error::Unauthorized { challenge, .. }) => {
                self.handle_unauthorized(challenge.as_ref()).await?;
                let result = self.request_once("initialize", params).await?;
                self.apply_initialize_result(&result)?;
                self.notify("notifications/initialized", json!({})).await?;
                Ok(())
            }
            Err(Error::Protocol { error, .. })
                if error.contains("400") || error.contains("404") || error.contains("405") =>
            {
                self.try_legacy_http_sse().await
            }
            Err(err) => Err(err),
        }
    }

    fn apply_initialize_result(&mut self, result: &Value) -> Result<(), Error> {
        if let Some(version) = result.get("protocolVersion").and_then(Value::as_str) {
            if version != MCP_PROTOCOL_VERSION
                && version != "2025-03-26"
                && version != "2025-06-18"
            {
                return Err(Error::Protocol {
                    server: self.server_name.clone(),
                    error: format!("unsupported MCP protocol version from server: {version}"),
                });
            }
            self.protocol_version = version.to_string();
        }
        Ok(())
    }

    async fn try_legacy_http_sse(&mut self) -> Result<(), Error> {
        debug!(
            server = %self.server_name,
            "attempting legacy HTTP+SSE MCP transport fallback"
        );
        let mut req = self
            .http
            .get(self.endpoint.clone())
            .header(ACCEPT, "text/event-stream");
        req = self.apply_common_headers(req)?;
        let response = req.send().await.map_err(|source| Error::Http {
            server: self.server_name.clone(),
            source,
        })?;
        if !response.status().is_success() {
            return Err(Error::Protocol {
                server: self.server_name.clone(),
                error: format!(
                    "legacy HTTP+SSE GET failed: HTTP {}",
                    response.status()
                ),
            });
        }
        let (events, _) = self
            .read_sse_until(response, None, Duration::from_secs(30), false)
            .await?;
        let endpoint_event = events.iter().find(|e| {
            e.event
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("endpoint"))
                || parse_json_data(&e.data).is_none() && !e.data.trim().is_empty()
        });
        let Some(ev) = endpoint_event else {
            return Err(Error::Protocol {
                server: self.server_name.clone(),
                error: "legacy HTTP+SSE stream missing endpoint event".to_string(),
            });
        };
        let message_url = self.endpoint.join(ev.data.trim()).map_err(|err| Error::Protocol {
            server: self.server_name.clone(),
            error: format!("invalid legacy endpoint URL: {err}"),
        })?;
        self.legacy_message_url = Some(message_url);
        let params = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "agent_Kuibyshev",
                "version": "0.1.0"
            }
        });
        let result = self.request_once("initialize", params).await?;
        self.apply_initialize_result(&result)?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    async fn handle_unauthorized(&mut self, challenge: Option<&BearerChallenge>) -> Result<(), Error> {
        let oauth = self.oauth.as_mut().ok_or_else(|| Error::Unauthorized {
            server: self.server_name.clone(),
            challenge: challenge.cloned(),
        })?;
        let step_up = challenge
            .and_then(|c| c.error.as_deref())
            .is_some_and(|e| e == "insufficient_scope");
        if step_up {
            // Force interactive re-auth with challenged scopes by clearing access token expiry.
            let _ = oauth.ensure_access_token(challenge).await?;
        } else {
            let _ = oauth.ensure_access_token(challenge).await?;
        }
        Ok(())
    }

    async fn request_once(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let started = Instant::now();
        let max_deadline = started + self.timeout.saturating_mul(3).max(self.timeout);

        let response = self.post_json(&payload).await?;
        let status = response.status();

        if status == StatusCode::UNAUTHORIZED {
            let challenge = parse_www_authenticate(response.headers());
            return Err(Error::Unauthorized {
                server: self.server_name.clone(),
                challenge,
            });
        }
        if status == StatusCode::NOT_FOUND && self.session_id.is_some() {
            return Err(Error::SessionExpired {
                server: self.server_name.clone(),
            });
        }
        if !(status.is_success() || status == StatusCode::ACCEPTED) {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Protocol {
                server: self.server_name.clone(),
                error: format!("HTTP {status}: {body}"),
            });
        }

        if let Some(session) = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session.to_string());
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if content_type.contains("text/event-stream") || content_type.contains("event-stream") {
            let remaining = max_deadline.saturating_duration_since(Instant::now());
            let (_events, result) = self
                .read_sse_until(response, Some(id), remaining, true)
                .await?;
            return result.ok_or_else(|| Error::Protocol {
                server: self.server_name.clone(),
                error: format!("SSE stream ended without JSON-RPC response for id {id}"),
            });
        }

        // Some servers omit/mislabel Content-Type; detect SSE by body prefix.
        let bytes = response.bytes().await.map_err(|source| Error::Http {
            server: self.server_name.clone(),
            source,
        })?;
        let as_text = String::from_utf8_lossy(&bytes);
        if as_text.starts_with("event:")
            || as_text.starts_with("data:")
            || as_text.starts_with("id:")
            || as_text.starts_with(':')
        {
            let mut parser = SseParser::new();
            let mut result = None;
            for event in parser.push(&as_text) {
                if let Some(value) = parse_json_data(&event.data) {
                    if value.get("id").and_then(Value::as_u64) == Some(id) {
                        result = Some(extract_result(&self.server_name, id, &value)?);
                        break;
                    }
                }
            }
            if let Some(event) = parser.finish() {
                if result.is_none() {
                    if let Some(value) = parse_json_data(&event.data) {
                        if value.get("id").and_then(Value::as_u64) == Some(id) {
                            result = Some(extract_result(&self.server_name, id, &value)?);
                        }
                    }
                }
            }
            return result.ok_or_else(|| Error::Protocol {
                server: self.server_name.clone(),
                error: format!(
                    "SSE-like body without JSON-RPC response for id {id} (content-type: {content_type})"
                ),
            });
        }

        let message: Value = serde_json::from_slice(&bytes).map_err(|err| Error::Protocol {
            server: self.server_name.clone(),
            error: format!(
                "failed to decode JSON-RPC body (content-type: {content_type}): {err}"
            ),
        })?;
        extract_result(&self.server_name, id, &message)
    }

    async fn post_json(&mut self, payload: &Value) -> Result<Response, Error> {
        let url = self
            .legacy_message_url
            .clone()
            .unwrap_or_else(|| self.endpoint.clone());
        let mut req = self.http.post(url).json(payload);
        req = req.header(ACCEPT, "application/json, text/event-stream");
        req = req.header(CONTENT_TYPE, "application/json");
        if self.session_id.is_some() || self.legacy_message_url.is_some() {
            // After initialize, always send negotiated protocol version.
            req = req.header(HEADER_PROTOCOL_VERSION, self.protocol_version.as_str());
        } else if payload.get("method").and_then(Value::as_str) != Some("initialize") {
            req = req.header(HEADER_PROTOCOL_VERSION, self.protocol_version.as_str());
        }
        if let Some(session) = &self.session_id {
            req = req.header(HEADER_SESSION_ID, session);
        }
        req = self.apply_common_headers(req)?;

        timeout(self.timeout, req.send())
            .await
            .map_err(|_| Error::Timeout {
                server: self.server_name.clone(),
                method: payload
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
            })?
            .map_err(|source| Error::Http {
                server: self.server_name.clone(),
                source,
            })
    }

    fn apply_common_headers(
        &self,
        mut req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, Error> {
        for (name, value) in self.static_headers.iter() {
            req = req.header(name, value);
        }
        if let Some(oauth) = &self.oauth {
            if let Some(token) = oauth.access_token() {
                let mut map = HeaderMap::new();
                apply_bearer(&mut map, token).map_err(|err| match err {
                    Error::OAuth { error, .. } => Error::OAuth {
                        server: self.server_name.clone(),
                        error,
                    },
                    other => other,
                })?;
                if let Some(value) = map.get(reqwest::header::AUTHORIZATION) {
                    req = req.header(reqwest::header::AUTHORIZATION, value);
                }
            }
        }
        Ok(req)
    }

    async fn read_sse_until(
        &self,
        response: Response,
        target_id: Option<u64>,
        overall_timeout: Duration,
        resume_on_disconnect: bool,
    ) -> Result<(Vec<crate::mcp::sse::SseEvent>, Option<Value>), Error> {
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        let mut last_event_id = None::<String>;
        let mut last_retry = Duration::from_secs(1);
        let mut result = None;
        let deadline = Instant::now() + overall_timeout;
        let mut stream = response.bytes_stream();

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout {
                    server: self.server_name.clone(),
                    method: "sse".to_string(),
                });
            }
            let next = timeout(remaining, stream.next()).await;
            match next {
                Ok(Some(Ok(chunk))) => {
                    let text = String::from_utf8_lossy(&chunk);
                    for event in parser.push(&text) {
                        if let Some(retry) = event.retry_ms {
                            last_retry = Duration::from_millis(retry);
                        }
                        if let Some(id) = &event.id {
                            last_event_id = Some(id.clone());
                        }
                        if let Some(value) = parse_json_data(&event.data) {
                            if let Some(tid) = target_id {
                                if value.get("id").and_then(Value::as_u64) == Some(tid) {
                                    result = Some(extract_result(
                                        &self.server_name,
                                        tid,
                                        &value,
                                    )?);
                                } else if value.get("method").is_some() {
                                    // progress / logging notifications — ignore for tools client
                                    debug!(
                                        server = %self.server_name,
                                        method = ?value.get("method"),
                                        "ignoring MCP SSE notification"
                                    );
                                }
                            }
                        }
                        events.push(event);
                        if result.is_some() {
                            return Ok((events, result));
                        }
                    }
                }
                Ok(Some(Err(err))) => {
                    return Err(Error::Http {
                        server: self.server_name.clone(),
                        source: err,
                    });
                }
                Ok(None) => {
                    if let Some(event) = parser.finish() {
                        events.push(event);
                    }
                    if result.is_some() || !resume_on_disconnect || target_id.is_none() {
                        return Ok((events, result));
                    }
                    // Resume via GET + Last-Event-ID
                    tokio::time::sleep(last_retry).await;
                    let mut req = self
                        .http
                        .get(self.endpoint.clone())
                        .header(ACCEPT, "text/event-stream")
                        .header(HEADER_PROTOCOL_VERSION, self.protocol_version.as_str());
                    if let Some(session) = &self.session_id {
                        req = req.header(HEADER_SESSION_ID, session);
                    }
                    if let Some(id) = &last_event_id {
                        req = req.header(HEADER_LAST_EVENT_ID, id);
                    }
                    req = self.apply_common_headers(req)?;
                    let response = req.send().await.map_err(|source| Error::Http {
                        server: self.server_name.clone(),
                        source,
                    })?;
                    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
                        return Ok((events, result));
                    }
                    if !response.status().is_success() {
                        return Err(Error::Protocol {
                            server: self.server_name.clone(),
                            error: format!(
                                "SSE resume GET failed: HTTP {}",
                                response.status()
                            ),
                        });
                    }
                    stream = response.bytes_stream();
                    parser = SseParser::new();
                }
                Err(_) => {
                    return Err(Error::Timeout {
                        server: self.server_name.clone(),
                        method: "sse".to_string(),
                    });
                }
            }
        }
    }
}

fn build_static_headers(
    server: &str,
    headers: &HashMap<String, String>,
) -> Result<HeaderMap, Error> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| Error::Protocol {
            server: server.to_string(),
            error: format!("invalid header name `{name}`: {err}"),
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|err| Error::Protocol {
            server: server.to_string(),
            error: format!("invalid header value for `{name}`: {err}"),
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn extract_result(server: &str, target_id: u64, message: &Value) -> Result<Value, Error> {
    let Some(id_value) = message.get("id") else {
        return Err(Error::Protocol {
            server: server.to_string(),
            error: "JSON-RPC response missing id".to_string(),
        });
    };
    if id_value.as_u64() != Some(target_id)
        && id_value.as_i64() != Some(target_id as i64)
        && id_value.as_str().and_then(|s| s.parse().ok()) != Some(target_id)
    {
        return Err(Error::Protocol {
            server: server.to_string(),
            error: format!("JSON-RPC response id mismatch (expected {target_id})"),
        });
    }
    if let Some(err) = message.get("error") {
        return Err(Error::Protocol {
            server: server.to_string(),
            error: err.to_string(),
        });
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

/// Test helper: build client without initialize (wiremock tests drive initialize themselves).
#[cfg(test)]
impl McpHttpClient {
    pub async fn connect_uninitialized_for_test(
        server_name: &str,
        url: &str,
        headers: HashMap<String, String>,
        auth: Option<crate::config::McpOAuthConfig>,
        timeout_ms: u64,
    ) -> Result<Self, Error> {
        let cfg = McpHttpConfig {
            url: url.to_string(),
            headers,
            auth,
        };
        let endpoint = Url::parse(cfg.url.trim()).map_err(|err| Error::Protocol {
            server: server_name.to_string(),
            error: format!("invalid mcp url: {err}"),
        })?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .no_proxy()
            .build()
            .map_err(|source| Error::Http {
                server: server_name.to_string(),
                source,
            })?;
        let static_headers = build_static_headers(server_name, &cfg.headers)?;
        let oauth = match cfg.auth {
            Some(auth) => Some(McpOAuth::new(server_name, endpoint.clone(), auth)?),
            None => None,
        };
        Ok(Self {
            server_name: server_name.to_string(),
            endpoint,
            timeout: Duration::from_millis(timeout_ms),
            http,
            static_headers,
            session_id: None,
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            next_id: AtomicU64::new(1),
            oauth,
            legacy_message_url: None,
        })
    }

    pub async fn initialize_for_test(&mut self) -> Result<(), Error> {
        self.initialize().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn jsonrpc_method(req: &Request) -> (u64, String) {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let id = body.get("id").and_then(Value::as_u64).unwrap_or(0);
        let method = body
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        (id, method)
    }

    #[tokio::test]
    async fn initialize_and_list_tools_json() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(|req: &Request| {
                let (id, method) = jsonrpc_method(req);
                match method.as_str() {
                    "initialize" => ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .insert_header("mcp-session-id", "sess-1")
                        .set_body_json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2025-11-25",
                                "capabilities": { "tools": {} },
                                "serverInfo": { "name": "mock", "version": "0" }
                            }
                        })),
                    "notifications/initialized" => ResponseTemplate::new(202),
                    "tools/list" => ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": [{ "name": "echo" }]
                            }
                        })),
                    other => ResponseTemplate::new(500).set_body_string(format!("unexpected {other}")),
                }
            })
            .mount(&server)
            .await;

        let mut client = McpHttpClient::connect_uninitialized_for_test(
            "remote",
            &format!("{}/mcp", server.uri()),
            HashMap::new(),
            None,
            5_000,
        )
        .await
        .expect("client");
        client.initialize_for_test().await.expect("init");
        let tools = client.list_tools().await.expect("tools");
        assert_eq!(tools, vec!["echo".to_string()]);
    }

    #[tokio::test]
    async fn tools_call_accepts_sse_response() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(|req: &Request| {
                let (id, method) = jsonrpc_method(req);
                match method.as_str() {
                    "initialize" => ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .insert_header("mcp-session-id", "s")
                        .set_body_json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2025-11-25",
                                "capabilities": {},
                                "serverInfo": { "name": "mock", "version": "0" }
                            }
                        })),
                    "notifications/initialized" => ResponseTemplate::new(202),
                    "tools/call" => {
                        let sse = format!(
                            "event: message\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{}}}}\n\n\
                             id: 9\ndata: {{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n\n"
                        );
                        ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream")
                    }
                    other => ResponseTemplate::new(500).set_body_string(format!("unexpected {other}")),
                }
            })
            .mount(&server)
            .await;

        let mut client = McpHttpClient::connect_uninitialized_for_test(
            "remote",
            &format!("{}/mcp", server.uri()),
            HashMap::new(),
            None,
            5_000,
        )
        .await
        .expect("client");
        client.initialize_for_test().await.expect("init");
        let result = client
            .request("tools/call", json!({"name": "echo", "arguments": {}}))
            .await
            .expect("call");
        assert_eq!(result["content"][0]["text"], "hi");
    }

    #[tokio::test]
    async fn oauth_token_exchange_then_initialize() {
        let server = MockServer::start().await;
        let token_store = tempfile::NamedTempFile::new().expect("tmp");

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-xyz",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token": "refresh-xyz"
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/mcp"))
            .and(header("authorization", "Bearer access-xyz"))
            .respond_with(|req: &Request| {
                let (id, method) = jsonrpc_method(req);
                match method.as_str() {
                    "initialize" => ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .insert_header("mcp-session-id", "authed")
                        .set_body_json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2025-11-25",
                                "capabilities": {},
                                "serverInfo": { "name": "secure", "version": "1" }
                            }
                        })),
                    "notifications/initialized" => ResponseTemplate::new(202),
                    other => ResponseTemplate::new(500).set_body_string(format!("unexpected {other}")),
                }
            })
            .mount(&server)
            .await;

        let auth = crate::config::McpOAuthConfig {
            client_id: Some("test-client".to_string()),
            client_secret_env: None,
            client_id_metadata_url: None,
            scopes: vec!["mcp:tools".to_string()],
            redirect_port: 0,
            token_store: Some(token_store.path().to_path_buf()),
        };
        let mut client = McpHttpClient::connect_uninitialized_for_test(
            "secure",
            &format!("{}/mcp", server.uri()),
            HashMap::new(),
            Some(auth),
            5_000,
        )
        .await
        .expect("client");

        {
            let oauth = client.oauth.as_mut().expect("oauth");
            oauth
                .exchange_authorization_code_for_test(
                    &format!("{}/token", server.uri()),
                    "test-client",
                    "auth-code",
                    "http://127.0.0.1/callback",
                    "verifier",
                    &format!("{}/mcp", server.uri()),
                )
                .await
                .expect("token");
        }

        client.initialize_for_test().await.expect("init with bearer");
    }
}
