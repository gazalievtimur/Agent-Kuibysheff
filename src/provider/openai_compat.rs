use std::env;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::sleep;

use crate::config::ProviderConfig;
use crate::limits::TokenUsage;
use crate::provider::{ChatMessage, ModelClient, ModelResponse};

#[derive(Debug, Error)]
pub enum ProviderError {
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

#[derive(Clone)]
pub struct OpenAiCompatClient {
    client: Client,
    cfg: ProviderConfig,
    api_key: String,
}

impl OpenAiCompatClient {
    pub fn new(cfg: ProviderConfig) -> Result<Self, ProviderError> {
        let api_key = env::var(&cfg.api_key_env)
            .map_err(|_| ProviderError::MissingApiKey(cfg.api_key_env.clone()))?;

        let client = Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .build()?;

        Ok(Self {
            client,
            cfg,
            api_key,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        )
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
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ModelResponse, ProviderError> {
        let body = ChatCompletionRequest {
            model: self.cfg.model.clone(),
            messages: messages.to_vec(),
            temperature: 0.0,
            stream: false,
        };

        for attempt in 0..=self.cfg.max_retries {
            let response = self
                .client
                .post(self.endpoint())
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;

            let response = match response {
                Ok(v) => v,
                Err(err) => {
                    if attempt == self.cfg.max_retries {
                        return Err(ProviderError::Http(err));
                    }
                    sleep(self.backoff_with_jitter(attempt)).await;
                    continue;
                }
            };

            let status = response.status();
            let text = response.text().await?;
            if !status.is_success() {
                let is_retryable =
                    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                if is_retryable && attempt < self.cfg.max_retries {
                    sleep(self.backoff_with_jitter(attempt)).await;
                    continue;
                }
                return Err(ProviderError::HttpStatus { status, body: text });
            }

            let payload: ChatCompletionResponse = serde_json::from_str(&text)?;
            let choice = payload
                .choices
                .into_iter()
                .next()
                .ok_or(ProviderError::EmptyChoices)?;
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

        unreachable!("retry loop always returns")
    }
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

#[derive(Debug, Clone, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
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
struct ChatUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}
