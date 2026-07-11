pub mod openai_compat;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::limits::TokenUsage;

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
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub content: String,
    pub usage: TokenUsage,
}

#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn complete(
        &self,
        messages: &[ChatMessage],
    ) -> Result<ModelResponse, openai_compat::ProviderError>;
}
