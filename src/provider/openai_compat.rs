use std::collections::BTreeMap;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rand::Rng;
use reqwest::header::HeaderMap;
use reqwest::redirect::{Attempt, Policy};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::sleep;
use tracing::{instrument, warn};

use crate::billing::{decimal_from_value, BillableMetric, ProviderAttemptAccounting, ReportedCost};
use crate::config::ProviderConfig;
use crate::limits::TokenUsage;
use crate::provider::{
    AccountedModelResponse, AccountedProviderError, ChatMessage, Error, ModelClient, ModelResponse,
};

/// Provider response accounting extraction options.
#[derive(Debug, Clone)]
pub struct ProviderAccountingOptions {
    pub provider_id: String,
    pub reported_cost_unit: Option<String>,
    pub cost_json_pointers: Vec<String>,
    pub cost_headers: Vec<String>,
}

impl Default for ProviderAccountingOptions {
    fn default() -> Self {
        Self {
            provider_id: "openai_compatible".to_string(),
            reported_cost_unit: None,
            cost_json_pointers: vec![
                "/usage/cost".to_string(),
                "/usage/response_cost/total_cost".to_string(),
            ],
            cost_headers: vec!["x-litellm-response-cost".to_string()],
        }
    }
}

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
    accounting: ProviderAccountingOptions,
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
        Self::new_with_accounting(cfg, ProviderAccountingOptions::default())
    }

    /// Builds a provider client with explicit billing metadata extraction.
    ///
    /// # Errors
    ///
    /// Returns the same validation/transport setup errors as [`Self::new`].
    pub fn new_with_accounting(
        cfg: ProviderConfig,
        accounting: ProviderAccountingOptions,
    ) -> Result<Self, Error> {
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
            accounting,
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

    fn accounting_attempt(&self, draft: AttemptDraft) -> ProviderAttemptAccounting {
        ProviderAttemptAccounting {
            attempt: draft.attempt,
            provider_id: self.accounting.provider_id.clone(),
            requested_model: self.cfg.model.clone(),
            resolved_model: draft.resolved_model,
            service_tier: draft.service_tier,
            request_id: draft.request_id,
            http_status: draft.status.map(|value| value.as_u16()),
            occurred_at_ms: draft.occurred_at_ms,
            usage_reported: draft.usage_reported,
            usage: draft.usage,
            billable_metrics: draft.billable_metrics,
            reported_cost: draft.reported_cost,
            error: draft.error,
        }
    }
}

struct AttemptDraft {
    attempt: u32,
    occurred_at_ms: u128,
    status: Option<StatusCode>,
    request_id: Option<String>,
    resolved_model: Option<String>,
    service_tier: Option<String>,
    usage: TokenUsage,
    usage_reported: bool,
    billable_metrics: BTreeMap<BillableMetric, u64>,
    reported_cost: Option<ReportedCost>,
    error: Option<String>,
}

#[async_trait]
impl ModelClient for OpenAiCompatClient {
    // `self` is skipped so `api_key` on the client never appears in span fields.
    #[instrument(skip(self, messages), fields(model = %self.cfg.model, message_count = messages.len()))]
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ModelResponse, Error> {
        self.complete_accounted(messages)
            .await
            .map(|accounted| accounted.response)
            .map_err(|failure| failure.error)
    }

    // `self` is skipped so `api_key` on the client never appears in span fields.
    #[instrument(skip(self, messages), fields(model = %self.cfg.model, message_count = messages.len()))]
    async fn complete_accounted(
        &self,
        messages: &[ChatMessage],
    ) -> Result<AccountedModelResponse, AccountedProviderError> {
        let mut retry_index = 0;
        let mut attempts = Vec::new();
        loop {
            let body = ChatCompletionRequest {
                model: &self.cfg.model,
                messages,
                temperature: 0.0,
                stream: false,
            };

            let occurred_at_ms = now_ms();
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
                    let message = err.to_string();
                    attempts.push(self.accounting_attempt(AttemptDraft {
                        attempt: retry_index + 1,
                        occurred_at_ms,
                        status: None,
                        request_id: None,
                        resolved_model: None,
                        service_tier: None,
                        usage: TokenUsage::default(),
                        usage_reported: false,
                        billable_metrics: BTreeMap::new(),
                        reported_cost: None,
                        error: Some(message),
                    }));
                    if retry_index >= self.cfg.max_retries {
                        return Err(AccountedProviderError {
                            error: Error::Http(err),
                            attempts,
                        });
                    }
                    sleep(self.backoff_with_jitter(retry_index)).await;
                    retry_index += 1;
                    continue;
                }
            };

            let status = response.status();
            let headers = response.headers().clone();
            let header_request_id = request_id_from_headers(&headers);
            let text = match response.text().await {
                Ok(text) => text,
                Err(err) => {
                    attempts.push(self.accounting_attempt(AttemptDraft {
                        attempt: retry_index + 1,
                        occurred_at_ms,
                        status: Some(status),
                        request_id: header_request_id,
                        resolved_model: None,
                        service_tier: None,
                        usage: TokenUsage::default(),
                        usage_reported: false,
                        billable_metrics: BTreeMap::new(),
                        reported_cost: None,
                        error: Some(err.to_string()),
                    }));
                    return Err(AccountedProviderError {
                        error: Error::Http(err),
                        attempts,
                    });
                }
            };
            let payload_parse = serde_json::from_str::<Value>(&text);
            let extracted = payload_parse
                .as_ref()
                .map(extract_usage)
                .unwrap_or_default();
            let reported_cost =
                extract_reported_cost(payload_parse.as_ref().ok(), &headers, &self.accounting);
            let response_request_id = payload_parse
                .as_ref()
                .ok()
                .and_then(|value| string_at(value, "/id"))
                .or(header_request_id);
            let resolved_model = payload_parse
                .as_ref()
                .ok()
                .and_then(|value| string_at(value, "/model"));
            let service_tier = payload_parse
                .as_ref()
                .ok()
                .and_then(|value| string_at(value, "/service_tier"));

            if !status.is_success() {
                attempts.push(self.accounting_attempt(AttemptDraft {
                    attempt: retry_index + 1,
                    occurred_at_ms,
                    status: Some(status),
                    request_id: response_request_id,
                    resolved_model,
                    service_tier,
                    usage: extracted.usage,
                    usage_reported: extracted.reported,
                    billable_metrics: extracted.metrics,
                    reported_cost,
                    error: Some(format!("provider returned HTTP {status}")),
                }));
                let is_retryable =
                    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                if is_retryable && retry_index < self.cfg.max_retries {
                    warn!(
                        status = %status,
                        attempt = retry_index,
                        "retrying provider request"
                    );
                    sleep(self.backoff_with_jitter(retry_index)).await;
                    retry_index += 1;
                    continue;
                }
                return Err(AccountedProviderError {
                    error: Error::HttpStatus { status, body: text },
                    attempts,
                });
            }

            let payload_value = match payload_parse {
                Ok(payload) => payload,
                Err(error) => {
                    attempts.push(self.accounting_attempt(AttemptDraft {
                        attempt: retry_index + 1,
                        occurred_at_ms,
                        status: Some(status),
                        request_id: response_request_id,
                        resolved_model,
                        service_tier,
                        usage: TokenUsage::default(),
                        usage_reported: false,
                        billable_metrics: BTreeMap::new(),
                        reported_cost,
                        error: Some(error.to_string()),
                    }));
                    return Err(AccountedProviderError {
                        error: Error::Decode(error),
                        attempts,
                    });
                }
            };
            let payload: ChatCompletionResponse = match serde_json::from_value(payload_value) {
                Ok(payload) => payload,
                Err(error) => {
                    attempts.push(self.accounting_attempt(AttemptDraft {
                        attempt: retry_index + 1,
                        occurred_at_ms,
                        status: Some(status),
                        request_id: response_request_id,
                        resolved_model,
                        service_tier,
                        usage: extracted.usage,
                        usage_reported: extracted.reported,
                        billable_metrics: extracted.metrics,
                        reported_cost,
                        error: Some(error.to_string()),
                    }));
                    return Err(AccountedProviderError {
                        error: Error::Decode(error),
                        attempts,
                    });
                }
            };
            let choice = match payload.choices.into_iter().next() {
                Some(choice) => choice,
                None => {
                    attempts.push(self.accounting_attempt(AttemptDraft {
                        attempt: retry_index + 1,
                        occurred_at_ms,
                        status: Some(status),
                        request_id: response_request_id,
                        resolved_model,
                        service_tier,
                        usage: extracted.usage,
                        usage_reported: extracted.reported,
                        billable_metrics: extracted.metrics,
                        reported_cost,
                        error: Some("provider response has no choices".to_string()),
                    }));
                    return Err(AccountedProviderError {
                        error: Error::EmptyChoices,
                        attempts,
                    });
                }
            };
            let content = extract_content(choice.message.content);
            attempts.push(self.accounting_attempt(AttemptDraft {
                attempt: retry_index + 1,
                occurred_at_ms,
                status: Some(status),
                request_id: response_request_id,
                resolved_model,
                service_tier,
                usage: extracted.usage,
                usage_reported: extracted.reported,
                billable_metrics: extracted.metrics,
                reported_cost,
                error: None,
            }));

            return Ok(AccountedModelResponse {
                response: ModelResponse {
                    content,
                    usage: extracted.usage,
                },
                attempts,
            });
        }
    }
}

#[derive(Debug, Default)]
struct ExtractedUsage {
    usage: TokenUsage,
    reported: bool,
    metrics: BTreeMap<BillableMetric, u64>,
}

fn extract_usage(payload: &Value) -> ExtractedUsage {
    let usage = payload
        .get("usage")
        .or_else(|| payload.get("usageMetadata"));
    let Some(usage) = usage else {
        return ExtractedUsage::default();
    };

    let aggregate_prompt = u64_at_any(
        usage,
        &["/prompt_tokens", "/promptTokenCount", "/prompt_token_count"],
    );
    let anthropic_input = u64_at_any(usage, &["/input_tokens"]);
    let cached = u64_at_any(
        usage,
        &[
            "/prompt_tokens_details/cached_tokens",
            "/input_tokens_details/cached_tokens",
            "/cache_read_input_tokens",
            "/cachedContentTokenCount",
            "/cached_content_token_count",
        ],
    )
    .unwrap_or_default();
    let cache_write = u64_at_any(
        usage,
        &[
            "/prompt_tokens_details/cache_write_tokens",
            "/cache_creation_input_tokens",
        ],
    )
    .unwrap_or_default();
    let prompt_tokens = if let Some(prompt_tokens) = aggregate_prompt {
        prompt_tokens
    } else {
        anthropic_input
            .unwrap_or_default()
            .saturating_add(cached)
            .saturating_add(cache_write)
    };

    let visible_output = u64_at_any(
        usage,
        &[
            "/completion_tokens",
            "/output_tokens",
            "/candidatesTokenCount",
            "/candidates_token_count",
            "/responseTokenCount",
        ],
    )
    .unwrap_or_default();
    let reasoning = u64_at_any(
        usage,
        &[
            "/completion_tokens_details/reasoning_tokens",
            "/output_tokens_details/reasoning_tokens",
            "/thoughtsTokenCount",
            "/thoughts_token_count",
        ],
    )
    .unwrap_or_default();
    let native_thinking_is_separate =
        usage.get("thoughtsTokenCount").is_some() || usage.get("thoughts_token_count").is_some();
    let completion_tokens = if native_thinking_is_separate {
        visible_output.saturating_add(reasoning)
    } else {
        visible_output
    };
    let total_tokens = u64_at_any(
        usage,
        &["/total_tokens", "/totalTokenCount", "/total_token_count"],
    )
    .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens));

    let uncached_input = if aggregate_prompt.is_some() {
        prompt_tokens
            .saturating_sub(cached)
            .saturating_sub(cache_write)
    } else {
        anthropic_input.unwrap_or(prompt_tokens)
    };
    let audio_input = u64_at_any(
        usage,
        &[
            "/prompt_tokens_details/audio_tokens",
            "/input_tokens_details/audio_tokens",
        ],
    )
    .unwrap_or_default();
    let image_input = u64_at_any(
        usage,
        &[
            "/prompt_tokens_details/image_tokens",
            "/input_tokens_details/image_tokens",
        ],
    )
    .unwrap_or_default();
    let audio_output = u64_at_any(
        usage,
        &[
            "/completion_tokens_details/audio_tokens",
            "/output_tokens_details/audio_tokens",
        ],
    )
    .unwrap_or_default();
    let image_output = u64_at_any(
        usage,
        &[
            "/completion_tokens_details/image_tokens",
            "/output_tokens_details/image_tokens",
        ],
    )
    .unwrap_or_default();
    let mut metrics = BTreeMap::new();
    metrics.insert(
        BillableMetric::InputTokens,
        uncached_input
            .saturating_sub(audio_input)
            .saturating_sub(image_input),
    );
    metrics.insert(BillableMetric::CachedInputTokens, cached);
    metrics.insert(BillableMetric::CacheWriteInputTokens, cache_write);
    metrics.insert(
        BillableMetric::OutputTokens,
        visible_output
            .saturating_sub(audio_output)
            .saturating_sub(image_output),
    );
    if native_thinking_is_separate {
        metrics.insert(BillableMetric::ReasoningTokens, reasoning);
    }
    insert_optional_metric(
        &mut metrics,
        BillableMetric::AudioInputTokens,
        (audio_input > 0).then_some(audio_input),
    );
    insert_optional_metric(
        &mut metrics,
        BillableMetric::ImageInputTokens,
        (image_input > 0).then_some(image_input),
    );
    insert_optional_metric(
        &mut metrics,
        BillableMetric::AudioOutputTokens,
        (audio_output > 0).then_some(audio_output),
    );
    insert_optional_metric(
        &mut metrics,
        BillableMetric::ImageOutputTokens,
        (image_output > 0).then_some(image_output),
    );
    insert_optional_metric(
        &mut metrics,
        BillableMetric::WebSearchRequests,
        u64_at_any(
            usage,
            &[
                "/server_tool_use/web_search_requests",
                "/server_tool_use_details/web_search_requests",
            ],
        ),
    );

    ExtractedUsage {
        usage: TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        },
        reported: true,
        metrics,
    }
}

fn extract_reported_cost(
    payload: Option<&Value>,
    headers: &HeaderMap,
    options: &ProviderAccountingOptions,
) -> Option<ReportedCost> {
    if let Some(payload) = payload {
        for pointer in &options.cost_json_pointers {
            let Some(value) = payload.pointer(pointer) else {
                continue;
            };
            if let Ok(amount) = decimal_from_value(value) {
                return Some(ReportedCost {
                    amount,
                    unit: options.reported_cost_unit.clone(),
                    source: format!("provider_json:{pointer}"),
                    details: extract_reported_cost_details(payload),
                });
            }
        }
    }
    for header in &options.cost_headers {
        let Some(value) = headers.get(header) else {
            continue;
        };
        let Ok(text) = value.to_str() else {
            continue;
        };
        if let Ok(amount) = crate::billing::parse_decimal(text) {
            return Some(ReportedCost {
                amount,
                unit: options.reported_cost_unit.clone(),
                source: format!("provider_header:{}", header.to_ascii_lowercase()),
                details: BTreeMap::new(),
            });
        }
    }
    None
}

fn extract_reported_cost_details(payload: &Value) -> BTreeMap<String, rust_decimal::Decimal> {
    [
        "/usage/cost_details",
        "/usage/response_cost",
        "/cost_details",
    ]
    .iter()
    .filter_map(|pointer| payload.pointer(pointer).and_then(Value::as_object))
    .flat_map(|object| object.iter())
    .filter_map(|(name, value)| {
        decimal_from_value(value)
            .ok()
            .map(|amount| (name.clone(), amount))
    })
    .collect()
}

fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    ["x-request-id", "request-id", "x-openai-request-id"]
        .iter()
        .find_map(|name| {
            headers
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned)
        })
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn u64_at_any(value: &Value, pointers: &[&str]) -> Option<u64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_u64))
}

fn insert_optional_metric(
    metrics: &mut BTreeMap<BillableMetric, u64>,
    metric: BillableMetric,
    quantity: Option<u64>,
) {
    if let Some(quantity) = quantity {
        metrics.insert(metric, quantity);
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::BillableMetric;
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

    #[test]
    fn gemini_usage_metadata_normalizes_cached_and_thinking_tokens() {
        let extracted = extract_usage(&serde_json::json!({
            "usageMetadata": {
                "promptTokenCount": 100,
                "cachedContentTokenCount": 40,
                "candidatesTokenCount": 5,
                "thoughtsTokenCount": 3,
                "totalTokenCount": 108
            }
        }));
        assert_eq!(extracted.usage.prompt_tokens, 100);
        assert_eq!(extracted.usage.completion_tokens, 8);
        assert_eq!(extracted.metrics[&BillableMetric::InputTokens], 60);
        assert_eq!(extracted.metrics[&BillableMetric::ReasoningTokens], 3);
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
    async fn accounted_response_preserves_tiny_cost_and_usage_breakdown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-request-id", "req-priced")
                    .set_body_json(serde_json::json!({
                        "id": "chat-priced",
                        "model": "resolved-model",
                        "service_tier": "default",
                        "choices": [{"message": {"content": "ok"}}],
                        "usage": {
                            "prompt_tokens": 100,
                            "completion_tokens": 7,
                            "total_tokens": 107,
                            "prompt_tokens_details": {"cached_tokens": 40},
                            "completion_tokens_details": {"reasoning_tokens": 3},
                            "cost": 0.00000894,
                            "cost_details": {
                                "upstream_inference_prompt_cost": 0.000004
                            }
                        }
                    })),
            )
            .mount(&server)
            .await;

        let client = OpenAiCompatClient::new_with_accounting(
            test_config(&format!("{}/v1", server.uri())),
            ProviderAccountingOptions {
                provider_id: "openrouter".to_string(),
                reported_cost_unit: Some("USD".to_string()),
                ..ProviderAccountingOptions::default()
            },
        )
        .expect("client");
        let accounted = client
            .complete_accounted(&[ChatMessage::new(ChatRole::User, "hi")])
            .await
            .expect("complete");

        assert_eq!(accounted.attempts.len(), 1);
        let attempt = &accounted.attempts[0];
        assert!(attempt.usage_reported);
        assert_eq!(attempt.request_id.as_deref(), Some("chat-priced"));
        assert_eq!(attempt.resolved_model.as_deref(), Some("resolved-model"));
        assert_eq!(attempt.billable_metrics[&BillableMetric::InputTokens], 60);
        assert_eq!(
            attempt.billable_metrics[&BillableMetric::CachedInputTokens],
            40
        );
        assert_eq!(
            attempt
                .reported_cost
                .as_ref()
                .expect("reported cost")
                .amount
                .to_string(),
            "0.00000894"
        );
        assert_eq!(
            attempt
                .reported_cost
                .as_ref()
                .expect("reported cost")
                .details["upstream_inference_prompt_cost"]
                .to_string(),
            "0.000004"
        );
    }

    #[tokio::test]
    async fn litellm_response_cost_header_is_supported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-litellm-response-cost", "8.94e-6")
                    .set_body_json(serde_json::json!({
                        "choices": [{"message": {"content": "ok"}}],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                    })),
            )
            .mount(&server)
            .await;

        let client =
            OpenAiCompatClient::new(test_config(&format!("{}/v1", server.uri()))).expect("client");
        let accounted = client
            .complete_accounted(&[ChatMessage::new(ChatRole::User, "hi")])
            .await
            .expect("complete");
        assert_eq!(
            accounted.attempts[0]
                .reported_cost
                .as_ref()
                .expect("header cost")
                .amount
                .to_string(),
            "0.00000894"
        );
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
        let failure = client
            .complete_accounted(&[ChatMessage::new(ChatRole::User, "hi")])
            .await
            .expect_err("should fail after retries");
        assert_eq!(failure.attempts.len(), 3);
        assert!(matches!(
            failure.error,
            Error::HttpStatus {
                status,
                ..
            } if status == StatusCode::INTERNAL_SERVER_ERROR
        ));
    }
}
