use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use thiserror::Error;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::warn;

use crate::output::LogReport;

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("failed to create log directory `{path}`: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open log file `{path}`: {source}")]
    OpenFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write log file `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode log record: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("log writer channel closed")]
    ChannelClosed,
}

#[derive(Clone)]
pub struct JsonlLogger {
    path: PathBuf,
    tx: mpsc::Sender<Vec<u8>>,
}

impl JsonlLogger {
    /// Opens or creates a JSONL log file for append-only writes.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError`] if the directory or file cannot be created or opened.
    pub async fn new(path: PathBuf) -> Result<Self, LoggingError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| LoggingError::CreateDir {
                    path: parent.display().to_string(),
                    source,
                })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|source| LoggingError::OpenFile {
                path: path.display().to_string(),
                source,
            })?;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let log_path = path.clone();
        tokio::spawn(async move {
            let mut file = file;
            while let Some(row) = rx.recv().await {
                if let Err(err) = file.write_all(&row).await {
                    warn!(
                        path = %log_path.display(),
                        error = %err,
                        "failed to write log record"
                    );
                }
            }
        });

        Ok(Self { path, tx })
    }

    /// Appends one JSONL event record to the log file.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError`] if serialization or enqueueing fails.
    pub async fn write_event<T: Serialize>(
        &self,
        event_type: &str,
        payload: &T,
    ) -> Result<(), LoggingError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let record = serde_json::json!({
            "ts_ms": ts,
            "event": event_type,
            "payload": payload,
        });
        let mut row = serde_json::to_vec(&record)?;
        row.push(b'\n');
        self.tx
            .send(row)
            .await
            .map_err(|_| LoggingError::ChannelClosed)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Default)]
pub struct Loggers {
    pub ai: Option<JsonlLogger>,
    pub mcp: Option<JsonlLogger>,
}

impl Loggers {
    /// Creates optional AI and MCP loggers based on runtime flags.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError`] if a requested log file cannot be opened.
    pub async fn from_flags(
        output_dir: Option<&PathBuf>,
        enable_ai_log: bool,
        enable_mcp_log: bool,
    ) -> Result<Self, LoggingError> {
        let fallback_dir = PathBuf::from("logs");
        let base_dir = output_dir.unwrap_or(&fallback_dir);
        let mut loggers = Self::default();
        if enable_ai_log {
            loggers.ai = Some(JsonlLogger::new(base_dir.join("ai_usage.jsonl")).await?);
        }
        if enable_mcp_log {
            loggers.mcp = Some(JsonlLogger::new(base_dir.join("mcp_usage.jsonl")).await?);
        }
        Ok(loggers)
    }

    #[must_use]
    pub fn report(&self) -> LogReport {
        LogReport {
            ai_log: self.ai.as_ref().map(|x| x.path().display().to_string()),
            mcp_log: self.mcp.as_ref().map(|x| x.path().display().to_string()),
        }
    }
}
