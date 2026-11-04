use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::output::{StopReason, UsageReport};
use crate::provider::ChatMessage;

use super::LoggingError;

pub const CHAT_HISTORY_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize)]
pub struct ChatHistoryRecord {
    pub schema_version: u32,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageReport>,
}

#[derive(Debug, Deserialize)]
struct ChatHistoryResumeFile {
    schema_version: u32,
    messages: Vec<ChatMessage>,
}

impl ChatHistoryRecord {
    #[must_use]
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            schema_version: CHAT_HISTORY_SCHEMA_VERSION,
            messages,
            result: None,
            stop_reason: None,
            usage: None,
        }
    }

    #[must_use]
    pub fn with_run_output(mut self, output: &crate::output::RunOutput) -> Self {
        self.result = Some(output.result.clone());
        self.stop_reason = Some(output.stop_reason.clone());
        self.usage = Some(output.usage.clone());
        self
    }
}

/// Writes the full chat transcript to a JSON file.
///
/// # Errors
///
/// Returns [`LoggingError`] when the directory cannot be created or the file
/// cannot be written.
pub async fn write_chat_history(
    path: &Path,
    record: &ChatHistoryRecord,
) -> Result<(), LoggingError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| LoggingError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
    }

    let payload = serde_json::to_vec_pretty(record)?;
    fs::write(path, payload)
        .await
        .map_err(|source| LoggingError::WriteFile {
            path: path.display().to_string(),
            source,
        })
}

/// Loads messages from a previously written chat transcript.
///
/// # Errors
///
/// Returns [`LoggingError`] when the file cannot be read or the schema/messages are invalid.
pub async fn load_chat_history(path: &Path) -> Result<Vec<ChatMessage>, LoggingError> {
    let payload = fs::read(path)
        .await
        .map_err(|source| LoggingError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
    let record: ChatHistoryResumeFile =
        serde_json::from_slice(&payload).map_err(|source| LoggingError::InvalidChatHistory {
            path: path.display().to_string(),
            reason: source.to_string(),
        })?;
    if record.schema_version != CHAT_HISTORY_SCHEMA_VERSION {
        return Err(LoggingError::InvalidChatHistory {
            path: path.display().to_string(),
            reason: format!(
                "unsupported schema_version {} (expected {CHAT_HISTORY_SCHEMA_VERSION})",
                record.schema_version
            ),
        });
    }
    if record.messages.is_empty() {
        return Err(LoggingError::InvalidChatHistory {
            path: path.display().to_string(),
            reason: "messages is empty".to_string(),
        });
    }
    Ok(record.messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatMessage, ChatRole};

    #[tokio::test]
    async fn write_chat_history_persists_messages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("chat_history.json");
        let record = ChatHistoryRecord::new(vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "hello"),
        ]);

        write_chat_history(&path, &record)
            .await
            .expect("write chat history");

        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(contents.contains("\"role\": \"system\""));
        assert!(contents.contains("hello"));

        let loaded = load_chat_history(&path).await.expect("load chat history");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].content.as_ref(), "hello");
    }

    #[tokio::test]
    async fn load_chat_history_rejects_empty_messages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("chat_history.json");
        std::fs::write(&path, r#"{"schema_version":2,"messages":[]}"#).expect("write");
        let err = load_chat_history(&path).await.expect_err("empty messages");
        assert!(err.to_string().contains("empty"));
    }
}
