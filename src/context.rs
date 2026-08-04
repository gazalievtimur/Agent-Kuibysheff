use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::access::InputFilesPolicy;

pub const MAX_INPUT_FILE_CHARS: usize = 50_000;
const MAX_INPUT_FILE_BYTES: usize = MAX_INPUT_FILE_CHARS * 4 + 4;

#[derive(Debug, Error)]
pub enum InputContextError {
    #[error("failed to read input file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("input file `{path}` denied: {reason}")]
    Denied { path: String, reason: String },
}

/// Builds a single context string from UTF-8 input files.
///
/// Each path is canonicalized, checked against [`InputFilesPolicy`], and deduplicated before
/// content is read into the prompt.
///
/// # Errors
///
/// Returns [`InputContextError`] if any input file cannot be read or is outside policy roots.
pub fn build_input_files_context(
    paths: &[PathBuf],
    policy: &InputFilesPolicy,
) -> Result<String, InputContextError> {
    let mut seen = BTreeSet::new();
    let mut sections = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = canonicalize_input(path, policy)?;
        let key = canonical.display().to_string();
        if !seen.insert(key) {
            continue;
        }
        sections.push(format_file(&canonical)?);
    }
    Ok(sections.join("\n\n"))
}

fn canonicalize_input(
    path: &Path,
    policy: &InputFilesPolicy,
) -> Result<PathBuf, InputContextError> {
    let canonical = fs::canonicalize(path).map_err(|source| InputContextError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    policy
        .allows_canonical_file(&canonical)
        .map_err(|reason| InputContextError::Denied {
            path: path.display().to_string(),
            reason,
        })?;
    Ok(canonical)
}

fn format_file(path: &Path) -> Result<String, InputContextError> {
    let (content, truncated_by_bytes) = read_utf8_prefix(path)?;
    let total_chars = content.chars().count();
    let truncated = truncated_by_bytes || total_chars > MAX_INPUT_FILE_CHARS;
    let content = if total_chars > MAX_INPUT_FILE_CHARS {
        content
            .chars()
            .take(MAX_INPUT_FILE_CHARS)
            .collect::<String>()
    } else {
        content
    };
    let marker = if truncated {
        "\n[truncated at 50000 characters]"
    } else {
        ""
    };

    Ok(format!(
        "--- file: {} ---\n{}{}\n--- end file ---",
        path.display(),
        content,
        marker
    ))
}

fn read_utf8_prefix(path: &Path) -> Result<(String, bool), InputContextError> {
    let file = File::open(path).map_err(|source| InputContextError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::with_capacity(MAX_INPUT_FILE_BYTES.min(8 * 1024));
    let mut chunk = [0u8; 8192];
    let mut truncated = false;

    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|source| InputContextError::ReadFile {
                path: path.display().to_string(),
                source,
            })?;
        if read == 0 {
            break;
        }
        let remaining = MAX_INPUT_FILE_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        let keep = remaining.min(read);
        bytes.extend_from_slice(&chunk[..keep]);
        if keep < read {
            truncated = true;
            break;
        }
    }

    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(err) => {
            let utf8_err = err.utf8_error();
            if truncated && utf8_err.error_len().is_none() {
                let mut bytes = err.into_bytes();
                bytes.truncate(utf8_err.valid_up_to());
                String::from_utf8(bytes).expect("valid UTF-8 prefix after truncation")
            } else {
                let source =
                    std::io::Error::new(std::io::ErrorKind::InvalidData, utf8_err.to_string());
                return Err(InputContextError::ReadFile {
                    path: path.display().to_string(),
                    source,
                });
            }
        }
    };

    Ok((content, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_file_path_and_content() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("input.md");
        fs::write(&path, "hello").expect("write input");

        let context =
            build_input_files_context(std::slice::from_ref(&path), &InputFilesPolicy::legacy())
                .expect("context");
        assert!(context.contains("hello"));
        assert!(context.contains(&fs::canonicalize(&path).unwrap().display().to_string()));
    }

    #[test]
    fn rejects_file_outside_input_roots() {
        let dir = tempfile::tempdir().expect("temp dir");
        let allowed = dir.path().join("inputs");
        let denied = dir.path().join("other");
        fs::create_dir_all(&allowed).expect("allowed");
        fs::create_dir_all(&denied).expect("denied");
        let allowed_file = allowed.join("a.txt");
        let denied_file = denied.join("b.txt");
        fs::write(&allowed_file, "ok").expect("write");
        fs::write(&denied_file, "no").expect("write");

        let policy = InputFilesPolicy::strict(vec![fs::canonicalize(&allowed).unwrap()]);
        build_input_files_context(std::slice::from_ref(&allowed_file), &policy).expect("allowed");
        let err = build_input_files_context(std::slice::from_ref(&denied_file), &policy)
            .expect_err("denied");
        assert!(matches!(err, InputContextError::Denied { .. }));
    }

    #[test]
    fn deduplicates_same_canonical_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("input.md");
        fs::write(&path, "once").expect("write");
        let same = dir.path().join(".").join("input.md");

        let context = build_input_files_context(&[path.clone(), same], &InputFilesPolicy::legacy())
            .expect("context");
        assert_eq!(context.matches("once").count(), 1);
    }

    #[test]
    fn truncates_large_utf8_file_without_broken_char() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("large.txt");
        let content = "🙂".repeat(MAX_INPUT_FILE_CHARS + 50);
        fs::write(&path, content).expect("write");

        let context =
            build_input_files_context(std::slice::from_ref(&path), &InputFilesPolicy::legacy())
                .expect("context");
        assert!(context.contains("[truncated at 50000 characters]"));
        assert!(!context.contains('\u{FFFD}'));
    }
}
