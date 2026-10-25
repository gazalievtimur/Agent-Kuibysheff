//! HTTP routes. The browser contract matches the previous in-process studio.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::jobs::{CancelJobError, JobManager, StartJobError, StartRunRequest};
use crate::kbshff::{envelope_data, Kbshff};
use crate::server::StudioState;

const STUDIO_HEADER: &str = "x-kbshff-studio";
const MAX_LOG_TAIL_BYTES: u64 = 256 * 1024;

fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({
            "error": message.into(),
        })),
    )
        .into_response()
}

fn studio_header_ok(headers: &HeaderMap) -> bool {
    headers
        .get(STUDIO_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some("1")
}

fn reject_missing_studio_header() -> Response {
    err(
        StatusCode::FORBIDDEN,
        "missing or invalid X-Kbshff-Studio header",
    )
}

fn client(state: &StudioState) -> Kbshff {
    Kbshff {
        bin: state.kbshff.clone(),
        project_root: state.project_root.clone(),
    }
}

async fn list_agents(State(state): State<StudioState>) -> Response {
    let kbshff = client(&state);
    let args = vec![
        "agent".to_string(),
        "list".to_string(),
        "--project-root".to_string(),
        state.project_root.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(value).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

#[derive(Deserialize)]
struct CreateAgentBody {
    id: String,
    #[serde(default)]
    force: bool,
}

async fn create_agent(
    State(state): State<StudioState>,
    headers: HeaderMap,
    Json(body): Json<CreateAgentBody>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let mut args = vec![
        "init".to_string(),
        body.id.clone(),
        "--project-root".to_string(),
        state.project_root.display().to_string(),
    ];
    if body.force {
        args.push("--force".to_string());
    }
    match client(&state).exec(&args, None).await {
        Ok(result) if result.success => Json(json!({
            "ok": true,
            "agent": body.id,
            "profile_dir": state
                .project_root
                .join(".kuibysheff")
                .join("protected")
                .join("agents")
                .join(&body.id)
                .display()
                .to_string(),
        }))
        .into_response(),
        Ok(result) => err(StatusCode::BAD_REQUEST, failure_text(&result)),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

#[derive(Deserialize)]
struct ImportAgentBody {
    id: String,
    from: PathBuf,
    #[serde(default)]
    force: bool,
}

async fn import_agent(
    State(state): State<StudioState>,
    headers: HeaderMap,
    Json(body): Json<ImportAgentBody>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    let mut args = kbshff.config_prefix(&body.id);
    args.push("import".to_string());
    args.push("--from".to_string());
    args.push(body.from.display().to_string());
    if body.force {
        args.push("--force".to_string());
    }
    match kbshff.exec_json(&args, None).await {
        Ok(_) => Json(json!({ "ok": true, "agent": body.id })).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn get_config(
    State(state): State<StudioState>,
    AxumPath(agent_id): AxumPath<String>,
) -> Response {
    let kbshff = client(&state);
    let mut args = kbshff.config_prefix(&agent_id);
    args.push("document".to_string());
    args.push("get".to_string());
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(envelope_data(value)).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn put_config(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    body: String,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    let mut args = kbshff.config_prefix(&agent_id);
    args.push("document".to_string());
    args.push("set".to_string());
    match kbshff.exec_json(&args, Some(body.into_bytes())).await {
        Ok(value) => Json(envelope_data(value)).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn get_prompt(
    State(state): State<StudioState>,
    AxumPath(agent_id): AxumPath<String>,
) -> Response {
    text_get(&state, &agent_id, "prompt", "master_prompt").await
}

async fn put_prompt(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    Json(body): Json<TextUpdate>,
) -> Response {
    text_put(&state, &headers, &agent_id, "prompt", &body.content).await
}

async fn get_rules(
    State(state): State<StudioState>,
    AxumPath(agent_id): AxumPath<String>,
) -> Response {
    text_get(&state, &agent_id, "rules", "rules").await
}

async fn put_rules(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    Json(body): Json<TextUpdate>,
) -> Response {
    text_put(&state, &headers, &agent_id, "rules", &body.content).await
}

#[derive(Deserialize)]
struct TextUpdate {
    content: String,
}

async fn text_get(state: &StudioState, agent_id: &str, command: &str, field: &str) -> Response {
    let kbshff = client(state);
    let mut args = kbshff.config_prefix(agent_id);
    args.push(command.to_string());
    args.push("get".to_string());
    match kbshff.exec_json(&args, None).await {
        Ok(value) => {
            let data = envelope_data(value);
            let content = data.get(field).and_then(Value::as_str).unwrap_or("");
            Json(json!({ "content": content })).into_response()
        }
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn text_put(
    state: &StudioState,
    headers: &HeaderMap,
    agent_id: &str,
    command: &str,
    content: &str,
) -> Response {
    if !studio_header_ok(headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(state);
    let mut args = kbshff.config_prefix(agent_id);
    args.push(command.to_string());
    args.push("set".to_string());
    args.push("--stdin".to_string());
    match kbshff
        .exec_json(&args, Some(content.as_bytes().to_vec()))
        .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn get_skills(
    State(state): State<StudioState>,
    AxumPath(agent_id): AxumPath<String>,
) -> Response {
    let kbshff = client(&state);
    let mut args = kbshff.config_prefix(&agent_id);
    args.push("skill".to_string());
    args.push("list".to_string());
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(envelope_data(value)).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn put_skills(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    body: String,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    let mut args = kbshff.config_prefix(&agent_id);
    args.push("skill".to_string());
    args.push("replace".to_string());
    match kbshff.exec_json(&args, Some(body.into_bytes())).await {
        Ok(value) => Json(envelope_data(value)).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

#[derive(Deserialize)]
struct SecretBody {
    key: String,
    value: String,
}

async fn put_secret(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    Json(body): Json<SecretBody>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    let mut args = kbshff.config_prefix(&agent_id);
    args.push("secret".to_string());
    args.push("set".to_string());
    args.push("--key".to_string());
    args.push(body.key);
    match kbshff.exec_json(&args, Some(body.value.into_bytes())).await {
        Ok(value) => Json(envelope_data(value)).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn run_check(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    let args = kbshff.identity_prefix("check", &agent_id);
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(value).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

#[derive(Deserialize, Default)]
struct McpDiscoverBody {
    #[serde(default)]
    server: Option<String>,
}

async fn mcp_discover(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    body: Option<Json<McpDiscoverBody>>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    let mut args = kbshff.identity_prefix("mcp", &agent_id);
    args.push("discover".to_string());
    if let Some(server) = body.and_then(|Json(body)| body.server) {
        args.push("--server".to_string());
        args.push(server);
    }
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(value).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

#[derive(Deserialize)]
struct McpCallBody {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: Value,
}

async fn mcp_call(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    Json(body): Json<McpCallBody>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    if body.server.trim().is_empty() || body.tool.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "server and tool are required");
    }
    let arguments = if body.arguments.is_null() {
        json!({})
    } else {
        body.arguments
    };
    let kbshff = client(&state);
    let mut args = kbshff.identity_prefix("mcp", &agent_id);
    args.push("call".to_string());
    args.push("--server".to_string());
    args.push(body.server);
    args.push("--tool".to_string());
    args.push(body.tool);
    let stdin = serde_json::to_vec(&arguments).unwrap_or_else(|_| b"{}".to_vec());
    match kbshff.exec_json(&args, Some(stdin)).await {
        Ok(value) => Json(value).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn list_out(
    State(state): State<StudioState>,
    AxumPath(agent_id): AxumPath<String>,
) -> Response {
    let kbshff = client(&state);
    let mut args = kbshff.identity_prefix("agent", &agent_id);
    // identity_prefix starts with the command name. `agent out` needs a subcommand.
    args.insert(1, "out".to_string());
    args.push("list".to_string());
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(value).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn get_out_file(
    State(state): State<StudioState>,
    AxumPath((agent_id, name)): AxumPath<(String, String)>,
) -> Response {
    let kbshff = client(&state);
    let mut args = kbshff.identity_prefix("agent", &agent_id);
    args.insert(1, "out".to_string());
    args.push("show".to_string());
    args.push(name);
    match kbshff.exec_json(&args, None).await {
        Ok(value) => Json(value).into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn start_run(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    Json(body): Json<StartRunRequest>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    let kbshff = client(&state);
    match state.jobs.start(&kbshff, agent_id, body).await {
        Ok(handle) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "run_id": handle.run_id,
                "agent_id": handle.agent_id,
                "status": "running",
            })),
        )
            .into_response(),
        Err(StartJobError::Conflict) => err(
            StatusCode::CONFLICT,
            "another studio run is already in progress",
        ),
        Err(StartJobError::Invalid(message)) => err(StatusCode::BAD_REQUEST, message),
    }
}

async fn get_run(State(state): State<StudioState>, AxumPath(run_id): AxumPath<String>) -> Response {
    match state.jobs.get(&run_id).await {
        Some(handle) => Json(handle.status_response().await).into_response(),
        None => err(StatusCode::NOT_FOUND, "run not found"),
    }
}

async fn cancel_run(
    State(state): State<StudioState>,
    headers: HeaderMap,
    AxumPath(run_id): AxumPath<String>,
) -> Response {
    if !studio_header_ok(&headers) {
        return reject_missing_studio_header();
    }
    match state.jobs.cancel(&run_id).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(CancelJobError::NotFound) => err(StatusCode::NOT_FOUND, "active run not found"),
    }
}

async fn run_events(
    State(state): State<StudioState>,
    AxumPath(run_id): AxumPath<String>,
) -> Response {
    let Some(handle) = state.jobs.get(&run_id).await else {
        return err(StatusCode::NOT_FOUND, "run not found");
    };
    let history = handle.history_snapshot().await;
    let rx = handle.subscribe();
    let hist_stream = stream::iter(
        history
            .into_iter()
            .map(|event| Ok::<Event, std::convert::Infallible>(event_to_sse(&event))),
    );
    let live = BroadcastStream::new(rx).filter_map(|item| match item {
        Ok(event) => Some(Ok::<Event, std::convert::Infallible>(event_to_sse(&event))),
        Err(_) => None,
    });
    let combined = hist_stream.chain(live);
    Sse::new(combined)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn event_to_sse(event: &Value) -> Event {
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    Event::default().data(data)
}

#[derive(Deserialize)]
struct LogQuery {
    which: String,
}

async fn run_logs(
    State(state): State<StudioState>,
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<LogQuery>,
) -> Response {
    let Some(handle) = state.jobs.get(&run_id).await else {
        return err(StatusCode::NOT_FOUND, "run not found");
    };
    let Some(path) = handle.log_path(&query.which).await else {
        return err(StatusCode::NOT_FOUND, "log not available yet");
    };
    if !path_inside_root(&state.project_root, &path) {
        return err(
            StatusCode::BAD_REQUEST,
            "log path is outside the project root",
        );
    }
    match read_tail(&path, MAX_LOG_TAIL_BYTES) {
        Ok(content) => Json(json!({
            "which": query.which,
            "path": path.display().to_string(),
            "content": content,
        }))
        .into_response(),
        Err(message) => err(StatusCode::BAD_REQUEST, message),
    }
}

fn path_inside_root(root: &Path, candidate: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(path) = candidate.canonicalize() else {
        return false;
    };
    path.starts_with(root)
}

fn read_tail(path: &Path, max_bytes: u64) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|err| err.to_string())?;
    let mut file = File::open(path).map_err(|err| err.to_string())?;
    if meta.len() > max_bytes {
        file.seek(SeekFrom::End(-(max_bytes as i64)))
            .map_err(|err| err.to_string())?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|err| err.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn failure_text(result: &crate::kbshff::ExecResult) -> String {
    let stderr = result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = result.stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    format!("kbshff exited with status {}", result.code.unwrap_or(-1))
}

#[derive(rust_embed::Embed)]
#[folder = "../../ui/studio/"]
struct StudioAssets;

async fn static_handler(AxumPath(path): AxumPath<String>) -> Response {
    serve_asset(&path)
}

async fn index_handler() -> Response {
    serve_asset("index.html")
}

fn serve_asset(path: &str) -> Response {
    let path = if path.is_empty() || path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };
    match StudioAssets::get(path) {
        Some(file) => {
            let mime = file.metadata.mimetype();
            let mut response = Response::new(Body::from(file.data.into_owned()));
            if let Ok(value) = HeaderValue::from_str(mime) {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            response
        }
        None => err(StatusCode::NOT_FOUND, "asset not found"),
    }
}

/// Build the axum router for the studio API and static UI.
pub fn router(state: StudioState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/", get(index_handler))
        .route("/assets/{*path}", get(static_handler))
        .route("/api/agents", get(list_agents).post(create_agent))
        .route("/api/agents/import", post(import_agent))
        .route("/api/agents/{id}/config", get(get_config).put(put_config))
        .route("/api/agents/{id}/prompt", get(get_prompt).put(put_prompt))
        .route("/api/agents/{id}/rules", get(get_rules).put(put_rules))
        .route("/api/agents/{id}/skills", get(get_skills).put(put_skills))
        .route("/api/agents/{id}/secret", post(put_secret))
        .route("/api/agents/{id}/check", post(run_check))
        .route("/api/agents/{id}/mcp/discover", post(mcp_discover))
        .route("/api/agents/{id}/mcp/call", post(mcp_call))
        .route("/api/agents/{id}/runs", post(start_run))
        .route("/api/agents/{id}/out", get(list_out))
        .route("/api/agents/{id}/out/{name}", get(get_out_file))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/runs/{id}/events", get(run_events))
        .route("/api/runs/{id}/cancel", post(cancel_run))
        .route("/api/runs/{id}/logs", get(run_logs))
        .route("/studio.css", get(|| async { serve_asset("studio.css") }))
        .route("/app.js", get(|| async { serve_asset("app.js") }))
        .with_state(state)
}

/// Router state helper so tests can supply a stub binary.
pub fn test_router(project_root: PathBuf, kbshff: PathBuf) -> axum::Router {
    router(StudioState {
        project_root,
        kbshff,
        jobs: JobManager::new(),
    })
}
