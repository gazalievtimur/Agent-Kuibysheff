//! Shared interactive terminal prompts for management commands.

use std::io::{BufRead, Write};

use thiserror::Error;

/// Prompt I/O failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PromptError {
    #[error("interactive prompt failed: {0}")]
    Io(String),
    #[error("invalid {label}: {message}")]
    Parse { label: String, message: String },
}

impl PromptError {
    #[must_use]
    pub fn io(err: impl ToString) -> Self {
        Self::Io(err.to_string())
    }
}

/// Ask for a line; empty input keeps `default`.
///
/// # Errors
///
/// Returns [`PromptError::Io`] on read/write failure.
pub fn prompt_string<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: &str,
) -> Result<String, PromptError> {
    write!(writer, "{label} [{default}]: ").map_err(PromptError::io)?;
    writer.flush().map_err(PromptError::io)?;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(PromptError::io)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Ask for a required non-empty line (no default).
///
/// # Errors
///
/// Returns [`PromptError`] when empty after trim or on I/O failure.
pub fn prompt_required<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> Result<String, PromptError> {
    write!(writer, "{label}: ").map_err(PromptError::io)?;
    writer.flush().map_err(PromptError::io)?;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(PromptError::io)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(PromptError::Parse {
            label: label.to_string(),
            message: "value must not be empty".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

/// Parse a value with a default; empty input keeps `default`.
///
/// # Errors
///
/// Returns [`PromptError`] on I/O or parse failure.
pub fn prompt_parse<R, W, T, E, F>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: T,
    parse: F,
) -> Result<T, PromptError>
where
    R: BufRead,
    W: Write,
    T: ToString + Copy,
    E: std::fmt::Display,
    F: Fn(&str) -> Result<T, E>,
{
    let default_text = default.to_string();
    let raw = prompt_string(reader, writer, label, &default_text)?;
    if raw == default_text {
        return Ok(default);
    }
    parse(&raw).map_err(|e| PromptError::Parse {
        label: label.to_string(),
        message: e.to_string(),
    })
}

/// Yes/no prompt. Empty input uses `default_yes`.
///
/// Accepts `y`/`yes` and `n`/`no` (case-insensitive).
///
/// # Errors
///
/// Returns [`PromptError`] on I/O failure or unrecognized answer.
pub fn prompt_yes_no<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default_yes: bool,
) -> Result<bool, PromptError> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    write!(writer, "{label} [{hint}]: ").map_err(PromptError::io)?;
    writer.flush().map_err(PromptError::io)?;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(PromptError::io)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(default_yes);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(PromptError::Parse {
            label: label.to_string(),
            message: format!("expected y/n, got `{other}`"),
        }),
    }
}
