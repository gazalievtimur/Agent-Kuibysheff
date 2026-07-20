use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{debug, instrument, warn};

use crate::config::ProviderConfig;
use crate::limits::TokenUsage;
use crate::provider::{ChatMessage, Error, ModelClient, ModelResponse};

#[derive(Clone)]
pub struct OpenAiCompatClient {
    client: Client,
    cfg: ProviderConfig,
    api_key: String,
}

impl OpenAiCompatClient {
    /// Builds a provider client from config and resolves the API key from
    /// `provider.api_key`, a `.env` file, or `provider.api_key_env`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::provider::Error`] if the API key is missing or the HTTP client cannot be built.
    pub fn new(cfg: ProviderConfig) -> Result<Self, Error> {
        let api_key = cfg
            .resolve_api_key()
            .map_err(|_| Error::MissingApiKey(cfg.api_key_env.clone()))?;

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
    #[instrument(skip(self, messages), fields(model = %self.cfg.model, message_count = messages.len()))]
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ModelResponse, Error> {
        for attempt in 0..=self.cfg.max_retries {
            let body = ChatCompletionRequest {
                model: &self.cfg.model,
                messages,
                temperature: 0.0,
                stream: false,
            };

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
                        return Err(Error::Http(err));
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
                    warn!(
                        status = %status,
                        attempt,
                        "retrying provider request"
                    );
                    sleep(self.backoff_with_jitter(attempt)).await;
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

        debug!("provider retry loop exhausted");
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
