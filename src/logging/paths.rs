use std::path::PathBuf;

use crate::config::LoggingConfig;

use super::LoggingError;

const DEFAULT_LOG_SUBDIR: &str = ".agent-kuibysheff/logs";

/// Returns the default log directory under the user's home folder.
///
/// # Errors
///
/// Returns [`LoggingError::HomeNotFound`] when neither `HOME` nor `USERPROFILE`
/// is set, and [`LoggingError::CreateDir`] when the directory cannot be created.
pub fn default_log_dir() -> Result<PathBuf, LoggingError> {
    let home = user_home_dir().ok_or(LoggingError::HomeNotFound)?;
    let dir = home.join(DEFAULT_LOG_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|source| LoggingError::CreateDir {
        path: dir.display().to_string(),
        source,
    })?;
    Ok(dir)
}

/// Resolves the base directory for log files from config and environment.
///
/// Priority:
/// 1. `AGENT_LOG_DIR` environment variable
/// 2. legacy `logging.output_dir`
/// 3. `logging.sink.path` for file sinks
/// 4. [`default_log_dir`]
///
/// # Errors
///
/// Returns [`LoggingError`] when the home directory cannot be resolved, the
/// directory cannot be created, or a database sink is requested but not yet
/// implemented.
pub fn resolve_base_dir(config: &LoggingConfig) -> Result<PathBuf, LoggingError> {
    if let Ok(dir) = std::env::var("AGENT_LOG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            ensure_dir(&path)?;
            return Ok(path);
        }
    }

    if let Some(output_dir) = &config.output_dir {
        ensure_dir(output_dir)?;
        return Ok(output_dir.clone());
    }

    match &config.sink {
        crate::config::LogSinkConfig::File { path: Some(path) } => {
            ensure_dir(path)?;
            Ok(path.clone())
        }
        crate::config::LogSinkConfig::File { path: None } => default_log_dir(),
        crate::config::LogSinkConfig::Db { .. } => Err(LoggingError::UnsupportedSink(
            "database sink is not implemented yet".to_string(),
        )),
    }
}

fn ensure_dir(path: &std::path::Path) -> Result<(), LoggingError> {
    std::fs::create_dir_all(path).map_err(|source| LoggingError::CreateDir {
        path: path.display().to_string(),
        source,
    })
}

fn user_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    if let Ok(profile) = std::env::var("USERPROFILE") {
        let trimmed = profile.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogSinkConfig;

    #[test]
    fn resolve_base_dir_prefers_output_dir_over_sink_path() {
        let legacy = tempfile::tempdir().expect("legacy tempdir");
        let sink = tempfile::tempdir().expect("sink tempdir");
        let config = LoggingConfig {
            enable_ai_log: true,
            enable_mcp_log: false,
            enable_chat_history: false,
            output_dir: Some(legacy.path().to_path_buf()),
            sink: LogSinkConfig::File {
                path: Some(sink.path().to_path_buf()),
            },
            ..Default::default()
        };

        assert_eq!(
            resolve_base_dir(&config).expect("resolve"),
            legacy.path().to_path_buf()
        );
    }

    #[test]
    fn resolve_base_dir_uses_sink_path_when_output_dir_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sink_path = dir.path().join("custom");
        let config = LoggingConfig {
            enable_ai_log: true,
            enable_mcp_log: false,
            enable_chat_history: false,
            output_dir: None,
            sink: LogSinkConfig::File {
                path: Some(sink_path.clone()),
            },
            ..Default::default()
        };

        assert_eq!(resolve_base_dir(&config).expect("resolve"), sink_path);
    }

    #[test]
    fn default_log_dir_uses_home_subdirectory() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());

        let log_dir = default_log_dir().expect("default log dir");
        assert_eq!(log_dir, dir.path().join(".agent-kuibysheff/logs"));
    }
}
