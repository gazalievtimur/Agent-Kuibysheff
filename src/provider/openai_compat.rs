use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use reqwest::redirect::{Attempt, Policy};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{instrument, warn};

use crate::config::ProviderConfig;
use crate::limits::TokenUsage;
use crate::provider::{ChatMessage, Error, ModelClient, ModelResponse};

/// Validated provider origin (`scheme` + `host` + effective `port`) from `provider.base_url`.
///
/// The model cannot change this URL during a run. HTTP calls from the main process go only here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedProviderOrigin {
    base: Url,
}

impl TrustedProviderOrigin {
    /// Parses and validates `provider.base_url`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBaseUrl`] when the URL is not an absolute `http`/`https` URL with a host.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidBaseUrl(
                "provider.base_url must not be empty".to_string(),
            ));
        }
        let base = Url::parse(trimmed)
            .map_err(|err| Error::InvalidBaseUrl(format!("failed to parse `{trimmed}`: {err}")))?;
        if base.scheme() != "http" && base.scheme() != "https" {
            return Err(Error::InvalidBaseUrl(format!(
                "provider.base_url must use http or https, got `{}`",
                base.scheme()
            )));
        }
        if base.host_str().is_none() {
            return Err(Error::InvalidBaseUrl(
                "provider.base_url must include a host".to_string(),
            ));
        }
        Ok(Self { base })
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Endpoint for chat completions on the trusted origin (same origin as `base_url`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBaseUrl`] if appending `/chat/completions` to the validated
    /// origin somehow yields an invalid URL.
    pub fn chat_completions_url(&self) -> Result<Url, Error> {
        self.join_path("chat/completions")
    }

    /// Endpoint for listing models on the trusted origin (same origin as `base_url`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBaseUrl`] if appending `/models` yields an invalid URL.
    pub fn models_url(&self) -> Result<Url, Error> {
        self.join_path("models")
    }

    fn join_path(&self, suffix: &str) -> Result<Url, Error> {
        let trimmed = self.base.as_str().trim_end_matches('/');
        Url::parse(&format!("{trimmed}/{suffix}")).map_err(|err| {
            Error::InvalidBaseUrl(format!(
                "failed to build `{suffix}` URL from `{trimmed}`: {err}"
            ))
        })
    }

    /// Whether `url` shares scheme, host, and effective port with this origin.
    #[must_use]
    pub fn same_origin(&self, url: &Url) -> bool {
        self.base.scheme() == url.scheme()
            && self.base.host() == url.host()
            && effective_port(&self.base) == effective_port(url)
    }
}

fn effective_port(url: &Url) -> Option<u16> {
    url.port_or_known_default()
}

#[derive(Clone)]
pub struct OpenAiCompatClient {
    client: Client,
    origin: TrustedProviderOrigin,
    endpoint: Url,
    cfg: ProviderConfig,
    api_key: String,
}

impl OpenAiCompatClient {
    /// Builds a provider client from config and resolves the API key from
    /// `provider.api_key`, a `.env` file, or `provider.api_key_env`.
    ///
    /// System HTTP(S) proxies are disabled. Redirects that leave the trusted origin are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`crate::provider::Error`] if the API key is missing, `base_url` is invalid, or the
    /// HTTP client cannot be built.
    pub fn new(cfg: ProviderConfig) -> Result<Self, Error> {
        let api_key = cfg
            .resolve_api_key()
            .map_err(|_| Error::MissingApiKey(cfg.api_key_env.clone()))?;
        let origin = TrustedProviderOrigin::parse(&cfg.base_url)?;
        let endpoint = origin.chat_completions_url()?;
        let redirect_origin = origin.clone();

        let client = Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .no_proxy()
            .redirect(Policy::custom(move |attempt: Attempt| {
                if redirect_origin.same_origin(attempt.url()) {
                    attempt.follow()
                } else {
                    let denied = attempt.url().clone();
                    attempt.error(format!(
                        "cross-origin redirect to `{denied}` denied by trusted provider origin"
                    ))
                }
            }))
            .build()?;

        Ok(Self {
            client,
            origin,
            endpoint,
            cfg,
            api_key,
        })
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn origin(&self) -> &TrustedProviderOrigin {
        &self.origin
    }

    /// Probes provider reachability and auth without spending chat tokens.
    ///
    /// Sends `GET {base_url}/models` with the configured API key. A successful
    /// response or a non-auth HTTP error that proves the host answered counts as
    /// reachable. Missing/invalid credentials (`401`/`403`) and transport
    /// failures are reported as errors.
    ///
    /// # Errors
    ///
    /// Returns [`crate::provider::Error`] when the request cannot be built, the
    /// host is unreachable, or authentication is rejected.
    pub async fn probe(&self) -> Result<String, Error> {
        let models_url = self.origin.models_url()?;
        let response = self
            .client
            .get(models_url.as_str())
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(format!("GET /models → {status}"));
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let snippet = truncate_body(&body, 200);
            return Err(Error::HttpStatus {
                status,
                body: snippet,
            });
        }
        // Host answered; /models may be unimplemented on some OpenAI-compatible APIs.
        Ok(format!(
            "host reachable (GET /models → {status}; chat endpoint may still work)"
        ))
    }

    fn backoff_with_jitter(&self, attempt: u32) -> Duration {
        let base = self.cfg.retry_base_delay_ms.max(1);
        let factor = 2u64.saturating_pow(attempt.min(10));
        let raw = base.saturating_mul(factor);
        let jitter: u64 = rand::thread_rng().gen_range(0..=raw / 4 + 1);
        Duration::from_millis(raw.saturating_add(jitter))
    }
}

#[async_trait]
impl ModelClient for OpenAiCompatClient {
    #[instrument(skip(self, messages), fields(model = %self.cfg.model, message_count = messages.len()))]
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ModelResponse, Error> {
        let mut attempt = 0;
        loop {
            let body = ChatCompletionRequest {
                model: &self.cfg.model,
                messages,
                temperature: 0.0,
                stream: false,
            };

            let response = self
                .client
                .post(self.endpoint.as_str())
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;

            let response = match response {
                Ok(v) => v,
                Err(err) => {
                    if attempt >= self.cfg.max_retries {
                        return Err(Error::Http(err));
                    }
                    sleep(self.backoff_with_jitter(attempt)).await;
                    attempt += 1;
                    continue;
                }
            };

            let status = response.status();
            let text = response.text().await?;
            if !status.is_success() {
                let is_retryable =
                    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                if is_retryable && attempt < self.cfg.max_retries {
                    warn!(
                        status = %status,
                        attempt,
                        "retrying provider request"
                    );
                    sleep(self.backoff_with_jitter(attempt)).await;
                    attempt += 1;
                    continue;
                }
                return Err(Error::HttpStatus { status, body: text });
            }

            let payload: ChatCompletionResponse = serde_json::from_str(&text)?;
            let choice = payload
                .choices
                .into_iter()
                .next()
                .ok_or(Error::EmptyChoices)?;
            let content = extract_content(choice.message.content);
            let usage = payload
                .usage
                .map(|x| TokenUsage {
                    prompt_tokens: x.prompt_tokens,
                    completion_tokens: x.completion_tokens,
                    total_tokens: x.total_tokens,
                })
                .unwrap_or_default();

            return Ok(ModelResponse { content, usage });
        }
    }
}

fn truncate_body(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max_chars).collect();
    format!("{truncated}…")
}

fn extract_content(content: MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text,
        MessageContent::Parts(parts) => parts
            .into_iter()
            .filter(|part| part.kind == "text")
            .map(|part| part.text.unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatAssistantMessage,
}

#[derive(Debug, Deserialize)]
struct ChatAssistantMessage {
    content: MessageContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_field_names)]
struct ChatUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ChatRole;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(base_url: &str) -> ProviderConfig {
        ProviderConfig {
            base_url: base_url.to_string(),
            model: "test-model".to_string(),
            api_key_env: "TEST_PROVIDER_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            timeout_ms: 5_000,
            max_retries: 0,
            retry_base_delay_ms: 1,
            history: crate::config::ProviderHistoryConfig::default(),
        }
    }

    #[test]
    fn trusted_origin_rejects_non_http() {
        let err = TrustedProviderOrigin::parse("ftp://example.com").expect_err("ftp");
        assert!(matches!(err, Error::InvalidBaseUrl(_)));
    }

    #[test]
    fn trusted_origin_same_origin_ignores_path() {
        let origin = TrustedProviderOrigin::parse("https://api.example.com/v1").unwrap();
        let same = Url::parse("https://api.example.com/v1/chat/completions").unwrap();
        let other_host = Url::parse("https://evil.example.com/v1/chat/completions").unwrap();
        let other_scheme = Url::parse("http://api.example.com/v1/chat/completions").unwrap();
        assert!(origin.same_origin(&same));
        assert!(!origin.same_origin(&other_host));
        assert!(!origin.same_origin(&other_scheme));
    }

    #[tokio::test]
    async fn follows_same_origin_redirect() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(307).insert_header(
                "Location",
                format!("{}/v1/chat/completions-final", server.uri()),
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions-final"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "ok"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            })))
            .mount(&server)
            .await;

        let client =
            OpenAiCompatClient::new(test_config(&format!("{}/v1", server.uri()))).expect("client");
        let response = client
            .complete(&[ChatMessage::new(ChatRole::User, "hi")])
            .await
            .expect("complete");
        assert_eq!(response.content, "ok");
    }

    #[tokio::test]
    async fn rejects_cross_origin_redirect() {
        let trusted = MockServer::start().await;
        let evil = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/steal", evil.uri())),
            )
            .mount(&trusted)
            .await;
        Mock::given(method("POST"))
            .and(path("/steal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "leaked"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            })))
            .mount(&evil)
            .await;

        let client =
            OpenAiCompatClient::new(test_config(&format!("{}/v1", trusted.uri()))).expect("client");
        let err = client
            .complete(&[ChatMessage::new(ChatRole::User, "hi")])
            .await
            .expect_err("cross-origin");
        let message = err.to_string();
        assert!(
            message.contains("redirect") || message.contains("error sending request"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn probe_accepts_successful_models_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "test-model"}]
            })))
            .mount(&server)
            .await;

        let client =
            OpenAiCompatClient::new(test_config(&format!("{}/v1", server.uri()))).expect("client");
        let detail = client.probe().await.expect("probe");
        assert!(detail.contains("200"), "{detail}");
    }

    #[tokio::test]
    async fn probe_rejects_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let client =
            OpenAiCompatClient::new(test_config(&format!("{}/v1", server.uri()))).expect("client");
        let err = client.probe().await.expect_err("unauthorized");
        assert!(matches!(
            err,
            Error::HttpStatus {
                status: StatusCode::UNAUTHORIZED,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn probe_treats_missing_models_endpoint_as_reachable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client =
            OpenAiCompatClient::new(test_config(&format!("{}/v1", server.uri()))).expect("client");
        let detail = client.probe().await.expect("probe");
        assert!(detail.contains("reachable"), "{detail}");
    }

    #[test]
    fn invalid_base_url_rejected_at_new() {
        let mut cfg = test_config("https://example.com");
        cfg.base_url = "not a valid url".to_string();
        let err = match OpenAiCompatClient::new(cfg) {
            Err(err) => err,
            Ok(_) => panic!("expected invalid base_url error"),
        };
        assert!(matches!(err, Error::InvalidBaseUrl(_)));
    }

    #[tokio::test]
    async fn retry_exhaustion_returns_http_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(3)
            .mount(&server)
            .await;

        let mut cfg = test_config(&format!("{}/v1", server.uri()));
        cfg.max_retries = 2;
        cfg.retry_base_delay_ms = 1;
        let client = OpenAiCompatClient::new(cfg).expect("client");
        let err = client
            .complete(&[ChatMessage::new(ChatRole::User, "hi")])
            .await
            .expect_err("should fail after retries");
        assert!(matches!(
            err,
            Error::HttpStatus {
                status,
                ..
            } if status == StatusCode::INTERNAL_SERVER_ERROR
        ));
    }
}
