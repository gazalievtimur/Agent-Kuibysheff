use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::access::InputFilesPolicy;

pub const MAX_INPUT_FILE_CHARS: usize = 50_000;

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
    let content = fs::read_to_string(path).map_err(|source| InputContextError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let total_chars = content.chars().count();
    let truncated = total_chars > MAX_INPUT_FILE_CHARS;
    let content = if truncated {
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
}
