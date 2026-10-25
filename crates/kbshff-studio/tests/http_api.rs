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
