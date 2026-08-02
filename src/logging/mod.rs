mod chat_history;
mod paths;
mod sink;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tracing::warn;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::LoggingConfig;
use crate::output::LogReport;
use crate::provider::ChatMessage;

pub use chat_history::{write_chat_history, ChatHistoryRecord};
pub use paths::{default_log_dir, resolve_base_dir};
pub use sink::{
    create_event_sink, create_file_sink, DbEventSink, EventSink, FailingEventSink, FileJsonlSink,
    JsonlLogger, MemoryEventSink, SharedEventSink, SinkDestination, TrackingEventSink,
};

use thiserror::Error;

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
    #[error("log writer task failed: {0}")]
    TaskJoin(String),
    #[error("user home directory is not set")]
    HomeNotFound,
    #[error("unsupported log sink: {0}")]
    UnsupportedSink(String),
}

#[derive(Clone, Default)]
pub struct Loggers {
    pub ai: Option<SharedEventSink>,
    pub mcp: Option<SharedEventSink>,
    system_log: Option<PathBuf>,
    chat_history_path: Option<PathBuf>,
    /// Set when any tracked audit sink `write_event` fails (runtime soft failures).
    audit_write_failed: Arc<AtomicBool>,
}

impl Loggers {
    /// Creates optional AI and MCP loggers from runtime logging configuration.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError`] if a requested sink cannot be opened.
    pub async fn from_config(config: &LoggingConfig) -> Result<Self, LoggingError> {
        let base_dir = resolve_base_dir(config)?;
        let audit_write_failed = Arc::new(AtomicBool::new(false));
        let mut loggers = Self {
            system_log: Some(base_dir.join("agent.trace.log")),
            audit_write_failed: audit_write_failed.clone(),
            ..Self::default()
        };

        if config.enable_ai_log {
            let sink = create_event_sink(config, "ai_usage.jsonl").await?;
            loggers.ai = Some(TrackingEventSink::wrap(sink, audit_write_failed.clone()));
        }
        if config.enable_mcp_log {
            let sink = create_event_sink(config, "mcp_usage.jsonl").await?;
            loggers.mcp = Some(TrackingEventSink::wrap(sink, audit_write_failed));
        }
        if config.enable_chat_history {
            loggers.chat_history_path = Some(base_dir.join("chat_history.json"));
        }

        Ok(loggers)
    }

    /// Builds loggers with explicit AI/MCP sinks (for tests and custom wiring).
    #[must_use]
    pub fn with_sinks(ai: Option<SharedEventSink>, mcp: Option<SharedEventSink>) -> Self {
        let audit_write_failed = Arc::new(AtomicBool::new(false));
        Self {
            ai: ai.map(|sink| TrackingEventSink::wrap(sink, audit_write_failed.clone())),
            mcp: mcp.map(|sink| TrackingEventSink::wrap(sink, audit_write_failed.clone())),
            audit_write_failed,
            ..Self::default()
        }
    }

    /// Whether any AI/MCP audit `write_event` has failed this run.
    #[must_use]
    pub fn audit_write_failed(&self) -> bool {
        self.audit_write_failed.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn chat_history_enabled(&self) -> bool {
        self.chat_history_path.is_some()
    }

    /// Persists the full chat transcript when chat history logging is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`LoggingError`] when the transcript file cannot be written.
    pub async fn save_chat_history(&self, record: &ChatHistoryRecord) -> Result<(), LoggingError> {
        let Some(path) = &self.chat_history_path else {
            return Ok(());
        };
        write_chat_history(path, record).await
    }

    /// Persists chat history and logs a warning instead of failing the run.
    pub async fn persist_chat_history(
        &self,
        messages: &[ChatMessage],
        output: Option<&crate::output::RunOutput>,
    ) {
        if !self.chat_history_enabled() {
            return;
        }

        let mut record = ChatHistoryRecord::new(messages.to_vec());
        if let Some(out) = output {
            record = record.with_run_output(out);
        }

        if let Err(err) = self.save_chat_history(&record).await {
            warn!(error = %err, "failed to save chat history");
        }
    }

    #[must_use]
    pub fn report(&self) -> LogReport {
        LogReport {
            ai_log: destination_path(self.ai.as_ref()),
            mcp_log: destination_path(self.mcp.as_ref()),
            system_log: self
                .system_log
                .as_ref()
                .map(|path| path.display().to_string()),
            chat_log: self
                .chat_history_path
                .as_ref()
                .map(|path| path.display().to_string()),
        }
    }

    /// Flushes any buffered records from active sinks and waits for writer tasks to finish.
    pub async fn shutdown(&self) {
        if let Some(ai) = &self.ai {
            if let Err(err) = ai.shutdown().await {
                warn!(error = %err, "failed to flush AI log sink");
            }
        }
        if let Some(mcp) = &self.mcp {
            if let Err(err) = mcp.shutdown().await {
                warn!(error = %err, "failed to flush MCP log sink");
            }
        }
    }

    #[must_use]
    pub fn system_log_path(&self) -> Option<&Path> {
        self.system_log.as_deref()
    }
}

fn destination_path(sink: Option<&SharedEventSink>) -> Option<String> {
    sink.and_then(|logger| match logger.destination() {
        SinkDestination::File(path) => Some(path.display().to_string()),
        SinkDestination::Database => None,
    })
}

/// Initializes tracing to stderr and an append-only trace file in the log directory.
///
/// # Errors
///
/// Returns [`LoggingError`] when the trace file cannot be created or opened.
pub fn init_tracing(log_dir: &Path) -> Result<PathBuf, LoggingError> {
    std::fs::create_dir_all(log_dir).map_err(|source| LoggingError::CreateDir {
        path: log_dir.display().to_string(),
        source,
    })?;

    let trace_path = log_dir.join("agent.trace.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&trace_path)
        .map_err(|source| LoggingError::OpenFile {
            path: trace_path.display().to_string(),
            source,
        })?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(FileMakeWriter(Arc::new(Mutex::new(file))));

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    Ok(trace_path)
}

#[derive(Clone)]
struct FileMakeWriter(Arc<Mutex<std::fs::File>>);

struct FileWriter(Arc<Mutex<std::fs::File>>);

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .flush()
    }
}

impl<'a> MakeWriter<'a> for FileMakeWriter {
    type Writer = FileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        FileWriter(Arc::clone(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogSinkConfig;

    #[tokio::test]
    async fn from_config_creates_enabled_channel_sinks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = LoggingConfig {
            enable_ai_log: true,
            enable_mcp_log: true,
            enable_chat_history: false,
            output_dir: Some(dir.path().to_path_buf()),
            sink: LogSinkConfig::default(),
        };

        let loggers = Loggers::from_config(&config).await.expect("loggers");
        assert!(loggers.ai.is_some());
        assert!(loggers.mcp.is_some());
        assert_eq!(
            loggers.system_log_path(),
            Some(dir.path().join("agent.trace.log").as_path())
        );

        let report = loggers.report();
        assert_eq!(
            report.ai_log,
            Some(dir.path().join("ai_usage.jsonl").display().to_string())
        );
        assert_eq!(
            report.mcp_log,
            Some(dir.path().join("mcp_usage.jsonl").display().to_string())
        );
        assert_eq!(
            report.system_log,
            Some(dir.path().join("agent.trace.log").display().to_string())
        );
    }

    #[tokio::test]
    async fn from_config_enables_chat_history_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = LoggingConfig {
            enable_ai_log: false,
            enable_mcp_log: false,
            enable_chat_history: true,
            output_dir: Some(dir.path().to_path_buf()),
            sink: LogSinkConfig::default(),
        };

        let loggers = Loggers::from_config(&config).await.expect("loggers");
        assert!(loggers.chat_history_enabled());
        assert_eq!(
            loggers.report().chat_log,
            Some(dir.path().join("chat_history.json").display().to_string())
        );
    }
}
