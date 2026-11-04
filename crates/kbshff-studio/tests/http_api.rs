//! Studio HTTP against a stub `kbshff` binary.

use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn app() -> axum::Router {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_kbshff_stub"));
    kbshff_studio::api_test_router(PathBuf::from("."), bin)
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn lists_agents_reads_config_and_rejects_check_without_header() {
    let app = app();
    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_json = body_json(list).await;
    assert_eq!(list_json["agents"], serde_json::json!(["demo"]));

    let cfg = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cfg.status(), StatusCode::OK);
    let cfg_json = body_json(cfg).await;
    assert_eq!(cfg_json["config"]["provider"]["model"], "m");
    assert_eq!(cfg_json["api_key_env"], "OPENAI_API_KEY");

    let forbidden = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/demo/check")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn second_run_conflicts_and_cancel_stops_the_child() {
    let app = app();
    let start = |app: axum::Router| {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/demo/runs")
                .header("content-type", "application/json")
                .header("x-kbshff-studio", "1")
                .body(Body::from(r#"{"prompt":"hello"}"#))
                .unwrap(),
        )
    };
    let first = start(app.clone()).await.unwrap();
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first_json = body_json(first).await;
    let run_id = first_json["run_id"].as_str().expect("run id").to_string();

    let second = start(app.clone()).await.unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{run_id}/cancel"))
                .header("x-kbshff-studio", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut status = String::from("running");
    while tokio::time::Instant::now() < deadline && status == "running" {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let polled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/runs/{run_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(polled).await;
        status = json["status"].as_str().unwrap_or("running").to_string();
    }
    assert_eq!(status, "cancelled");
}

#[tokio::test]
async fn skill_body_round_trip_uses_profile_command() {
    let app = app();
    let got = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/skills/research/body")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
    let got_json = body_json(got).await;
    assert_eq!(got_json["content"], "be careful");

    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/demo/skills/research/body")
                .header("content-type", "application/json")
                .header("x-kbshff-studio", "1")
                .body(Body::from(r#"{"content":"follow the skill"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    let missing_header = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/demo/skills/research/body")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"follow the skill"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_header.status(), StatusCode::FORBIDDEN);

    let cleared = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/demo/skills/research/body")
                .header("content-type", "application/json")
                .header("x-kbshff-studio", "1")
                .body(Body::from(r#"{"content":"  "}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cleared.status(), StatusCode::OK);

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/skills/missing/body")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let rejected = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/demo/skills/bad/body")
                .header("content-type", "application/json")
                .header("x-kbshff-studio", "1")
                .body(Body::from(r#"{"content":"nope"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn operator_routes_cover_writes_assets_and_missing_runs() {
    let app = app();

    let index = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(index.status(), StatusCode::OK);

    let css = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/studio.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(css.status(), StatusCode::OK);

    let missing_asset = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/missing.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_asset.status(), StatusCode::NOT_FOUND);

    let created = post_json(
        app.clone(),
        "/api/agents",
        true,
        r#"{"id":"demo","force":true}"#,
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    assert_eq!(body_json(created).await["agent"], "demo");

    let forbidden = post_json(app.clone(), "/api/agents", false, r#"{"id":"demo"}"#).await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let failed = post_json(app.clone(), "/api/agents", true, r#"{"id":"bad"}"#).await;
    assert_eq!(failed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(failed).await["error"], "init failed");

    let stdout_fail = post_json(app.clone(), "/api/agents", true, r#"{"id":"bad-stdout"}"#).await;
    assert_eq!(stdout_fail.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(stdout_fail).await["error"], "plain failure");

    let empty_fail = post_json(app.clone(), "/api/agents", true, r#"{"id":"bad-empty"}"#).await;
    assert_eq!(empty_fail.status(), StatusCode::BAD_REQUEST);

    let imported = post_json(
        app.clone(),
        "/api/agents/import",
        true,
        r#"{"id":"demo","from":"profile","force":true}"#,
    )
    .await;
    assert_eq!(imported.status(), StatusCode::OK);

    let config = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/demo/config")
                .header("content-type", "application/json")
                .header("x-kbshff-studio", "1")
                .body(Body::from(r#"{"config":{}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(config.status(), StatusCode::OK);

    let prompt = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/prompt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prompt.status(), StatusCode::OK);
    assert_eq!(body_json(prompt).await["content"], "sys");

    let prompt_put = post_text(
        app.clone(),
        "PUT",
        "/api/agents/demo/prompt",
        r#"{"content":"p"}"#,
    )
    .await;
    assert_eq!(prompt_put.status(), StatusCode::OK);

    let rules = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/rules")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rules.status(), StatusCode::OK);
    assert_eq!(body_json(rules).await["content"], "be brief");

    let rules_put = post_text(
        app.clone(),
        "PUT",
        "/api/agents/demo/rules",
        r#"{"content":"r"}"#,
    )
    .await;
    assert_eq!(rules_put.status(), StatusCode::OK);

    let skills = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(skills.status(), StatusCode::OK);

    let skills_put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/demo/skills")
                .header("content-type", "application/json")
                .header("x-kbshff-studio", "1")
                .body(Body::from(r#"{"skills":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(skills_put.status(), StatusCode::OK);

    let secret = post_json(
        app.clone(),
        "/api/agents/demo/secret",
        true,
        r#"{"key":"TOKEN","value":"x"}"#,
    )
    .await;
    assert_eq!(secret.status(), StatusCode::OK);

    let check = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/demo/check")
                .header("x-kbshff-studio", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(check.status(), StatusCode::OK);

    let discover = post_json(
        app.clone(),
        "/api/agents/demo/mcp/discover",
        true,
        r#"{"server":"local"}"#,
    )
    .await;
    assert_eq!(discover.status(), StatusCode::OK);

    let called = post_json(
        app.clone(),
        "/api/agents/demo/mcp/call",
        true,
        r#"{"server":"local","tool":"ping","arguments":{}}"#,
    )
    .await;
    assert_eq!(called.status(), StatusCode::OK);

    let missing_tool = post_json(
        app.clone(),
        "/api/agents/demo/mcp/call",
        true,
        r#"{"server":" ","tool":" ","arguments":null}"#,
    )
    .await;
    assert_eq!(missing_tool.status(), StatusCode::BAD_REQUEST);

    let out = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/out")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::OK);

    let out_file = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/demo/out/report")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(out_file.status(), StatusCode::OK);

    let missing_run = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_run.status(), StatusCode::NOT_FOUND);

    let missing_events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs/missing/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_events.status(), StatusCode::NOT_FOUND);

    let missing_cancel = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/missing/cancel")
                .header("x-kbshff-studio", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_cancel.status(), StatusCode::NOT_FOUND);

    let missing_logs = app
        .oneshot(
            Request::builder()
                .uri("/api/runs/missing/logs?which=stdout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_logs.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn empty_prompt_without_continue_is_rejected() {
    let app = app();
    let response = post_json(app, "/api/agents/demo/runs", true, r#"{"prompt":""}"#).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("prompt must not be empty"),
        "{json}"
    );
}

#[tokio::test]
async fn continue_without_chat_history_is_rejected() {
    let app = app();
    let response = post_json(
        app,
        "/api/agents/demo/runs",
        true,
        r#"{"continue_from_chat":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("chat history not found"),
        "{json}"
    );
}

#[tokio::test]
async fn continue_with_chat_history_is_accepted() {
    let dir = std::env::temp_dir().join(format!("kbshff-studio-continue-{}", std::process::id()));
    let logs = dir
        .join(".kuibysheff")
        .join("homes")
        .join("demo")
        .join("logs");
    std::fs::create_dir_all(&logs).expect("logs");
    std::fs::write(logs.join("chat_history.json"), "{}").expect("chat");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_kbshff_stub"));
    let app = kbshff_studio::api_test_router(dir.clone(), bin);
    let response = post_json(
        app.clone(),
        "/api/agents/demo/runs",
        true,
        r#"{"continue_from_chat":true}"#,
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let json = body_json(response).await;
    let run_id = json["run_id"].as_str().expect("run id");
    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{run_id}/cancel"))
                .header("x-kbshff-studio", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
}

async fn post_json(
    app: axum::Router,
    uri: &str,
    studio: bool,
    body: &str,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if studio {
        builder = builder.header("x-kbshff-studio", "1");
    }
    app.oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn post_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("x-kbshff-studio", "1")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}
