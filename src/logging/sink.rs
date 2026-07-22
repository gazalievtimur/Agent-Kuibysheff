use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::Value;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::{LogSinkConfig, LoggingConfig};

use super::paths::resolve_base_dir;
use super::LoggingError;

/// Where structured event records are persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkDestination {
    File(PathBuf),
    Database,
}

/// Abstraction for structured event log destinations.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Appends one structured event record.
    async fn write_event(&self, event_type: &str, payload: Value) -> Result<(), LoggingError>;

    /// Returns the logical destination for reporting.
    fn destination(&self) -> SinkDestination;

    /// Flushes any buffered records and releases background resources.
    async fn shutdown(&self) -> Result<(), LoggingError>;
}

pub type SharedEventSink = Arc<dyn EventSink>;

/// Append-only JSONL file sink.
pub struct FileJsonlSink {
    path: PathBuf,
    tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl FileJsonlSink {
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
        let handle = tokio::spawn(async move {
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

        Ok(Self {
            path,
            tx: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Closes the channel and waits for the background writer to finish.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError::TaskJoin`] if the background writer panics.
    pub async fn flush(self) -> Result<(), LoggingError> {
        self.shutdown().await
    }

    /// Closes the channel and waits for the background writer to finish.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError::ChannelClosed`] if the sink has already been shut down.
    /// Returns [`LoggingError::TaskJoin`] if the background writer panics.
    pub async fn shutdown(&self) -> Result<(), LoggingError> {
        let tx = self.tx.lock().unwrap().take();
        drop(tx);
        let handle = self.handle.lock().unwrap().take();
        let Some(handle) = handle else {
            return Err(LoggingError::ChannelClosed);
        };
        handle
            .await
            .map_err(|err| LoggingError::TaskJoin(err.to_string()))
    }
}

impl Drop for FileJsonlSink {
    fn drop(&mut self) {
        let tx = self.tx.lock().unwrap().take();
        drop(tx);
        let handle = self.handle.lock().unwrap().take();
        if let Some(handle) = handle {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                // Drain the writer from a temporary thread to avoid blocking the runtime thread
                // in current-thread schedulers or during runtime shutdown.
                let _ = std::thread::spawn(move || {
                    let _ = runtime.block_on(tokio::time::timeout(Duration::from_secs(5), handle));
                })
                .join();
            }
        }
    }
}

#[async_trait]
impl EventSink for FileJsonlSink {
    async fn write_event(&self, event_type: &str, payload: Value) -> Result<(), LoggingError> {
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
        let tx = self.tx.lock().unwrap().clone();
        let Some(tx) = tx else {
            return Err(LoggingError::ChannelClosed);
        };
        tx.send(row).await.map_err(|_| LoggingError::ChannelClosed)
    }

    fn destination(&self) -> SinkDestination {
        SinkDestination::File(self.path.clone())
    }

    async fn shutdown(&self) -> Result<(), LoggingError> {
        FileJsonlSink::shutdown(self).await
    }
}

/// Placeholder for a future database-backed sink.
#[derive(Clone)]
pub struct DbEventSink {
    connection_string: String,
}

impl DbEventSink {
    #[must_use]
    pub fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
        }
    }
}

#[async_trait]
impl EventSink for DbEventSink {
    async fn write_event(&self, _event_type: &str, _payload: Value) -> Result<(), LoggingError> {
        let _ = &self.connection_string;
        Err(LoggingError::UnsupportedSink(
            "database sink is not implemented yet".to_string(),
        ))
    }

    fn destination(&self) -> SinkDestination {
        SinkDestination::Database
    }

    async fn shutdown(&self) -> Result<(), LoggingError> {
        Ok(())
    }
}

/// In-memory sink for tests and diagnostics capture.
#[derive(Default)]
pub struct MemoryEventSink {
    events: Mutex<Vec<(String, Value)>>,
}

impl MemoryEventSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of recorded `(event_type, payload)` pairs.
    #[must_use]
    pub fn events(&self) -> Vec<(String, Value)> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventSink for MemoryEventSink {
    async fn write_event(&self, event_type: &str, payload: Value) -> Result<(), LoggingError> {
        self.events
            .lock()
            .unwrap()
            .push((event_type.to_string(), payload));
        Ok(())
    }

    fn destination(&self) -> SinkDestination {
        SinkDestination::Database
    }

    async fn shutdown(&self) -> Result<(), LoggingError> {
        Ok(())
    }
}

/// Backward-compatible alias for the file-backed JSONL sink.
pub type JsonlLogger = FileJsonlSink;

/// Creates an event sink for a named channel file inside the resolved base dir.
///
/// # Errors
///
/// Returns [`LoggingError`] when the target file cannot be opened.
pub async fn create_file_sink(
    base_dir: &Path,
    file_name: &str,
) -> Result<SharedEventSink, LoggingError> {
    let sink = FileJsonlSink::new(base_dir.join(file_name)).await?;
    Ok(Arc::new(sink))
}

/// Creates an event sink from logging configuration.
///
/// # Errors
///
/// Returns [`LoggingError`] when the sink cannot be opened or the requested
/// destination is not implemented.
pub async fn create_event_sink(
    config: &LoggingConfig,
    file_name: &str,
) -> Result<SharedEventSink, LoggingError> {
    match &config.sink {
        LogSinkConfig::File { .. } => {
            let base_dir = resolve_base_dir(config)?;
            create_file_sink(&base_dir, file_name).await
        }
        LogSinkConfig::Db { connection_string } => {
            Ok(Arc::new(DbEventSink::new(connection_string.clone())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LoggingConfig;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_sink_writes_jsonl_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        let sink = FileJsonlSink::new(path.clone()).await.expect("sink");

        sink.write_event("test_event", serde_json::json!({"ok": true}))
            .await
            .expect("write");

        sink.flush().await.expect("flush");

        let contents = std::fs::read_to_string(path).expect("read");
        assert!(contents.contains("\"event\":\"test_event\""));
        assert!(contents.contains("\"ok\":true"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_event_sink_uses_configured_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = LoggingConfig {
            enable_ai_log: true,
            enable_mcp_log: false,
            enable_chat_history: false,
            output_dir: Some(dir.path().to_path_buf()),
            sink: LogSinkConfig::default(),
        };

        let sink = create_event_sink(&config, "ai_usage.jsonl")
            .await
            .expect("sink");
        match sink.destination() {
            SinkDestination::File(path) => assert_eq!(path, dir.path().join("ai_usage.jsonl")),
            SinkDestination::Database => panic!("expected file sink"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_flushes_buffered_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        let sink = FileJsonlSink::new(path.clone()).await.expect("sink");

        for i in 0..10 {
            sink.write_event("batch_event", serde_json::json!({"i": i}))
                .await
                .expect("write");
        }

        // Drop without explicit flush; Drop should drain the channel and wait for the writer.
        drop(sink);

        let contents = std::fs::read_to_string(path).expect("read");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(
            lines.len(),
            10,
            "expected 10 JSONL records, got {}",
            lines.len()
        );
        for (i, line) in lines.iter().enumerate() {
            assert!(
                line.contains("batch_event"),
                "record {i} missing event type"
            );
            assert!(
                line.contains(&format!("\"i\":{i}")),
                "record {i} missing payload"
            );
        }
    }
}
