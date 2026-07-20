pub mod openai_compat;

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::limits::TokenUsage;

/// Provider-layer error returned by [`ModelClient`] implementations.
#[derive(Debug, Error)]
pub enum Error {
    #[error("missing provider API key in environment variable `{0}`")]
    MissingApiKey(String),
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

#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ModelResponse, Error>;
}
