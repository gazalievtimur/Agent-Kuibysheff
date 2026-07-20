use std::path::Path;

use serde::Serialize;
use tokio::fs;

use crate::output::{StopReason, UsageReport};
use crate::provider::ChatMessage;

use super::LoggingError;

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

impl ChatHistoryRecord {
    #[must_use]
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            schema_version: 1,
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

        let contents = std::fs::read_to_string(path).expect("read");
        assert!(contents.contains("\"role\": \"system\""));
        assert!(contents.contains("hello"));
    }
}
