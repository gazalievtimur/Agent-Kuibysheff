//! Optional Bearer auth middleware for A2A `/jsonrpc` and `/rest`.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;
use tracing::warn;

/// Shared expected Bearer token (never logged).
#[derive(Clone)]
pub struct BearerToken(Arc<str>);

impl BearerToken {
    /// Wrap an expected token value.
    #[must_use]
    pub fn new(token: impl AsRef<str>) -> Self {
        Self(Arc::from(token.as_ref()))
    }

    /// Read token from an environment variable.
    ///
    /// # Errors
    ///
    /// Returns an error when the variable is missing or empty/whitespace-only.
    pub fn from_env(var_name: &str) -> anyhow::Result<Self> {
        let value = std::env::var(var_name).map_err(|_| {
            anyhow::anyhow!("A2A --token-env `{var_name}` is not set in the environment")
        })?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            anyhow::bail!("A2A --token-env `{var_name}` is empty");
        }
        Ok(Self::new(trimmed))
    }

    fn matches(&self, candidate: &str) -> bool {
        constant_time_eq(candidate.as_bytes(), self.0.as_bytes())
    }
}

/// Parse `Authorization: Bearer <token>` (scheme match is case-insensitive per RFC 6750).
fn parse_bearer_token(header_value: &str) -> Option<&str> {
    let mut parts = header_value.splitn(2, ' ');
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    parts.next()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Axum middleware: require `Authorization: Bearer <token>`.
pub async fn require_bearer(
    axum::extract::State(expected): axum::extract::State<BearerToken>,
    request: Request,
    next: Next,
) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer_token)
        .is_some_and(|got| expected.matches(got));

    if !authorized {
        warn!("A2A rejected request: missing or invalid Bearer token");
        let mut response = (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        if let Ok(value) = HeaderValue::from_str("Bearer") {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        return response;
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    async fn call_with_auth(token: Option<&str>, expected: BearerToken) -> StatusCode {
        let app = Router::new().route("/rpc", get(ok_handler)).layer(
            axum::middleware::from_fn_with_state(expected, require_bearer),
        );

        let mut builder = HttpRequest::builder().uri("/rpc").method("GET");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let req = builder.body(Body::empty()).expect("request");

        let response = app.oneshot(req).await.expect("response");
        response.status()
    }

    #[test]
    fn from_env_reads_token() {
        std::env::set_var("A2A_TEST_TOKEN", "  secret-value  ");
        let token = BearerToken::from_env("A2A_TEST_TOKEN").expect("token");
        assert!(token.matches("secret-value"));
        std::env::remove_var("A2A_TEST_TOKEN");
    }

    #[test]
    fn from_env_rejects_missing() {
        std::env::remove_var("A2A_TEST_TOKEN_MISSING");
        assert!(BearerToken::from_env("A2A_TEST_TOKEN_MISSING").is_err());
    }

    #[test]
    fn from_env_rejects_empty() {
        std::env::set_var("A2A_TEST_TOKEN_EMPTY", "   ");
        assert!(BearerToken::from_env("A2A_TEST_TOKEN_EMPTY").is_err());
        std::env::remove_var("A2A_TEST_TOKEN_EMPTY");
    }

    #[tokio::test]
    async fn accepts_valid_bearer_token() {
        let status = call_with_auth(Some("secret-token"), BearerToken::new("secret-token")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_case_insensitive_scheme() {
        let app = Router::new().route("/rpc", get(ok_handler)).layer(
            axum::middleware::from_fn_with_state(BearerToken::new("secret-token"), require_bearer),
        );

        let req = HttpRequest::builder()
            .uri("/rpc")
            .method("GET")
            .header(header::AUTHORIZATION, "bearer secret-token")
            .body(Body::empty())
            .expect("request");

        let response = app.oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let status = call_with_auth(None, BearerToken::new("secret-token")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_wrong_token() {
        let status = call_with_auth(Some("wrong"), BearerToken::new("secret-token")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
