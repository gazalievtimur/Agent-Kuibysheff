//! Provider traits and shared chat types.
//!
//! Stable: [`ModelClient`], [`ChatMessage`], [`ModelResponse`], [`Error`].
//! The OpenAI-compatible HTTP adapter is `pub(crate)`.

pub(crate) mod openai_compat;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::billing::{BillableMetric, ProviderAttemptAccounting};
use crate::limits::TokenUsage;

/// Provider-layer error returned by [`ModelClient`] implementations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("missing provider API key in environment variable `{0}`")]
    MissingApiKey(String),
    #[error("invalid provider base_url: {0}")]
    InvalidBaseUrl(String),
    #[error("http transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("provider returned status {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },
    #[error("failed to decode provider response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("provider response has no choices")]
    EmptyChoices,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Arc<str>,
}

impl ChatMessage {
    #[must_use]
    pub fn new(role: ChatRole, content: impl Into<Arc<str>>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub content: String,
    pub usage: TokenUsage,
}

/// A completion plus accounting for every physical provider attempt.
#[derive(Debug)]
pub struct AccountedModelResponse {
    pub response: ModelResponse,
    pub attempts: Vec<ProviderAttemptAccounting>,
}

/// Provider failure that retains any retry-attempt accounting already observed.
#[derive(Debug, Error)]
#[error("{error}")]
pub struct AccountedProviderError {
    #[source]
    pub error: Error,
    pub attempts: Vec<ProviderAttemptAccounting>,
}

/// Object-safe model client; `async_trait` is required because native `async fn` in traits is not
/// dyn-compatible for `Arc<dyn ModelClient>`.
#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ModelResponse, Error>;

    /// Completes with per-attempt accounting. Existing clients receive a
    /// conservative compatibility record from their basic [`ModelResponse`].
    async fn complete_accounted(
        &self,
        messages: &[ChatMessage],
    ) -> Result<AccountedModelResponse, AccountedProviderError> {
        match self.complete(messages).await {
            Ok(response) => {
                let usage = response.usage;
                let mut billable_metrics = std::collections::BTreeMap::new();
                billable_metrics.insert(BillableMetric::InputTokens, usage.prompt_tokens);
                billable_metrics.insert(BillableMetric::OutputTokens, usage.completion_tokens);
                let attempt = ProviderAttemptAccounting {
                    attempt: 1,
                    provider_id: "custom".to_string(),
                    requested_model: "unknown".to_string(),
                    resolved_model: None,
                    service_tier: None,
                    request_id: None,
                    http_status: None,
                    occurred_at_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |duration| duration.as_millis()),
                    usage_reported: usage.total_tokens > 0,
                    usage,
                    billable_metrics,
                    reported_cost: None,
                    error: None,
                };
                Ok(AccountedModelResponse {
                    response,
                    attempts: vec![attempt],
                })
            }
            Err(error) => Err(AccountedProviderError {
                error,
                attempts: Vec::new(),
            }),
        }
    }
}
