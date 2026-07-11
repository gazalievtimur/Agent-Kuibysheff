use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub const MAX_INPUT_FILE_CHARS: usize = 50_000;

#[derive(Debug, Error)]
pub enum InputContextError {
    #[error("failed to read input file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Builds a single context string from UTF-8 input files.
///
/// # Errors
///
/// Returns [`InputContextError`] if any input file cannot be read.
pub fn build_input_files_context(paths: &[PathBuf]) -> Result<String, InputContextError> {
    let mut sections = Vec::with_capacity(paths.len());
    for path in paths {
        sections.push(format_file(path)?);
    }
    Ok(sections.join("\n\n"))
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

        let context = build_input_files_context(std::slice::from_ref(&path)).expect("context");
        assert!(context.contains(&path.display().to_string()));
        assert!(context.contains("hello"));
    }
}
