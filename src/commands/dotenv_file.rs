//! Read/update `.env` files next to an agent profile (secrets, not YAML).

use std::fs;
use std::io;
use std::path::Path;

use thiserror::Error;

/// Errors when updating a `.env` file.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DotenvFileError {
    #[error("failed to read `{path}`: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to write `{path}`: {source}")]
    Write {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("invalid env var name `{0}`: use `[A-Za-z_][A-Za-z0-9_]*`")]
    InvalidName(String),
}

/// Validate an environment variable name suitable for `api_key_env`.
///
/// # Errors
///
/// Returns [`DotenvFileError::InvalidName`] when the name is empty or malformed.
pub fn validate_env_var_name(name: &str) -> Result<(), DotenvFileError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(DotenvFileError::InvalidName(name.to_string()));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(DotenvFileError::InvalidName(name.to_string()));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DotenvFileError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// Create or update `KEY=value` in `path`, preserving other lines and comments.
///
/// # Errors
///
/// Returns I/O errors or an invalid key name.
pub fn upsert_env_var(path: &Path, key: &str, value: &str) -> Result<(), DotenvFileError> {
    validate_env_var_name(key)?;
    let path_display = path.display().to_string();
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(DotenvFileError::Read {
                path: path_display,
                source,
            });
        }
    };

    let assignment = format!("{key}={value}");
    let mut replaced = false;
    let mut out_lines: Vec<String> = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out_lines.push(line.to_string());
            continue;
        }
        let name = trimmed.split_once('=').map_or(trimmed, |(n, _)| n.trim());
        if name == key {
            out_lines.push(assignment.clone());
            replaced = true;
        } else {
            out_lines.push(line.to_string());
        }
    }
    if !replaced {
        if !out_lines.is_empty() && !out_lines.last().is_some_and(|l| l.is_empty()) {
            // keep single trailing newline style via join below
        }
        out_lines.push(assignment);
    }

    let mut body = out_lines.join("\n");
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| DotenvFileError::Write {
            path: path_display.clone(),
            source,
        })?;
    }
    fs::write(path, body).map_err(|source| DotenvFileError::Write {
        path: path_display,
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_bad_env_names() {
        assert!(validate_env_var_name("").is_err());
        assert!(validate_env_var_name("1ABC").is_err());
        assert!(validate_env_var_name("HAS-DASH").is_err());
        assert!(validate_env_var_name("OPENAI_API_KEY").is_ok());
        assert!(validate_env_var_name("_PRIVATE").is_ok());
    }

    #[test]
    fn creates_and_updates_without_clobbering_neighbors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".env");
        upsert_env_var(&path, "OPENAI_API_KEY", "first").unwrap();
        upsert_env_var(&path, "OTHER", "keep").unwrap();
        upsert_env_var(&path, "OPENAI_API_KEY", "second").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("OPENAI_API_KEY=second"));
        assert!(text.contains("OTHER=keep"));
        assert!(!text.contains("OPENAI_API_KEY=first"));
    }

    #[test]
    fn preserves_comments() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "# comment\nFOO=1\n").unwrap();
        upsert_env_var(&path, "FOO", "2").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# comment\n"));
        assert!(text.contains("FOO=2"));
    }
}
