use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;
use tokio::task;
use tracing::debug;

use crate::mcp::Error;

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "logs", ".cursor"];
const SKIP_EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "dylib", "png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "zip", "gz",
    "7z", "tar", "rar", "woff", "woff2", "ttf", "otf", "bin", "lock", "pyc", "class", "o", "a",
    "rlib",
];
const MAX_SEARCH_DEPTH: usize = 8;
const MAX_SEARCH_FILES: usize = 600;
/// Byte-size gate (conservative vs UTF-8 char counts) for files considered by search.
const MAX_SEARCH_FILE_BYTES: u64 = 250_000;
const DEFAULT_MAX_RESULTS: usize = 8;
const MIN_MAX_RESULTS: usize = 1;
const MAX_MAX_RESULTS: usize = 20;
const DEFAULT_READ_CHARS: usize = 6_000;
const MIN_READ_CHARS: usize = 100;
const MAX_READ_CHARS: usize = 200_000;
const MAX_SNIPPET_CHARS: usize = 300;

pub struct LocalTools {
    root: PathBuf,
}

impl LocalTools {
    /// Creates a repository-scoped toolset rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::mcp::Error`] if the directory cannot be resolved or is not a directory.
    pub async fn new(root: &Path) -> Result<Self, Error> {
        fs::create_dir_all(root)
            .await
            .map_err(|error| local_io("create_dir_all", root, &error))?;
        let root = fs::canonicalize(root)
            .await
            .map_err(|error| local_io("canonicalize", root, &error))?;
        if !fs::metadata(&root)
            .await
            .map_err(|error| local_io("metadata", &root, &error))?
            .is_dir()
        {
            return Err(local_path(root.display().to_string(), "root is not a directory"));
        }
        Ok(Self { root })
    }

    /// Dispatches a local repository tool call.
    ///
    /// # Errors
    ///
    /// Returns [`crate::mcp::Error`] for invalid arguments, paths, or I/O failures.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, Error> {
        match tool {
            "search_docs" => {
                let args: SearchDocsArgs = decode_args(tool, arguments)?;
                self.search_docs(&args.query, args.max_results).await
            }
            "read_file" => {
                let args: ReadFileArgs = decode_args(tool, arguments)?;
                self.read_file(Path::new(&args.path), args.max_chars).await
            }
            _ => Err(Error::UnknownTool {
                server: "local_tools".to_string(),
                tool: tool.to_string(),
            }),
        }
    }

    async fn search_docs(&self, query: &str, max_results: Option<usize>) -> Result<Value, Error> {
        let query = query.trim();
        if query.is_empty() {
            return Err(invalid_args(
                "local_tools.search_docs",
                "`query` must not be empty",
            ));
        }

        let max_results = clamp_usize(
            max_results,
            DEFAULT_MAX_RESULTS,
            MIN_MAX_RESULTS,
            MAX_MAX_RESULTS,
        );
        let root = self.root.clone();
        let root_display = root.display().to_string();
        let query = query.to_owned();

        task::spawn_blocking(move || search_docs_blocking(&root, &query, max_results))
            .await
            .map_err(|error| Error::LocalIo {
                operation: "spawn_blocking".to_string(),
                path: root_display,
                error: error.to_string(),
            })
    }

    async fn read_file(&self, relative: &Path, max_chars: Option<usize>) -> Result<Value, Error> {
        let relative_display = display_relative_input(relative);
        if relative_display.is_empty() {
            return Err(invalid_args(
                "local_tools.read_file",
                "`path` must not be empty",
            ));
        }

        let path = self.resolve_inside_root(relative).await?;
        let max_chars = clamp_usize(
            max_chars,
            DEFAULT_READ_CHARS,
            MIN_READ_CHARS,
            MAX_READ_CHARS,
        );

        let content = fs::read_to_string(&path)
            .await
            .map_err(|error| local_io("read_to_string", &path, &error))?;

        let total_chars = content.chars().count();
        let content = if total_chars > max_chars {
            format!(
                "{}\n...[truncated]",
                content.chars().take(max_chars).collect::<String>()
            )
        } else {
            content
        };

        Ok(json!({
            "path": display_relative_path(&self.root, &path),
            "content": content,
        }))
    }

    async fn resolve_inside_root(&self, relative: &Path) -> Result<PathBuf, Error> {
        validate_relative(relative)?;
        let candidate = self.root.join(relative);
        let canonical = fs::canonicalize(&candidate)
            .await
            .map_err(|error| local_io("canonicalize", &candidate, &error))?;
        ensure_within_root(&self.root, &canonical, relative)?;
        Ok(canonical)
    }
}

fn search_docs_blocking(root: &Path, query: &str, max_results: usize) -> Value {
    let query_lower = query.to_lowercase();
    let ascii_needle = query_lower.is_ascii();
    let mut matches = Vec::with_capacity(max_results.min(MAX_MAX_RESULTS));
    let mut files_seen = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        if matches.len() >= max_results || files_seen >= MAX_SEARCH_FILES {
            break;
        }
        if depth > MAX_SEARCH_DEPTH {
            continue;
        }

        let reader = match std::fs::read_dir(&dir) {
            Ok(reader) => reader,
            Err(error) => {
                debug!(path = %dir.display(), error = %error, "skipping unreadable search dir");
                continue;
            }
        };

        for entry in reader {
            if matches.len() >= max_results || files_seen >= MAX_SEARCH_FILES {
                break;
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    debug!(path = %dir.display(), error = %error, "skipping unreadable dir entry");
                    continue;
                }
            };

            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    debug!(path = %path.display(), error = %error, "skipping entry without file type");
                    continue;
                }
            };

            if file_type.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !SKIP_DIRS.contains(&name.as_ref()) {
                    stack.push((path, depth + 1));
                }
                continue;
            }

            if !file_type.is_file() || is_skipped_extension(&path) {
                continue;
            }

            files_seen = files_seen.saturating_add(1);
            if let Some(found) = match_file(root, &path, &query_lower, ascii_needle) {
                matches.push(found);
            }
        }
    }

    json!({
        "query": query,
        "total_matches": matches.len(),
        "matches": matches,
    })
}

fn match_file(
    root: &Path,
    path: &Path,
    query_lower: &str,
    ascii_needle: bool,
) -> Option<Value> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_SEARCH_FILE_BYTES {
        return None;
    }

    let text = std::fs::read_to_string(path).ok()?;
    for (idx, line) in text.lines().enumerate() {
        if line_matches(line, query_lower, ascii_needle) {
            return Some(json!({
                "file": display_relative_path(root, path),
                "line": idx + 1,
                "snippet": truncate_chars(line.trim(), MAX_SNIPPET_CHARS),
            }));
        }
    }
    None
}

fn line_matches(line: &str, query_lower: &str, ascii_needle: bool) -> bool {
    if ascii_needle && line.is_ascii() {
        contains_ignore_ascii_case(line, query_lower)
    } else {
        line.to_lowercase().contains(query_lower)
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut end = text.len();
    for (count, (idx, _)) in text.char_indices().enumerate() {
        if count == max_chars {
            end = idx;
            break;
        }
    }
    text[..end].to_owned()
}

#[derive(Deserialize)]
struct SearchDocsArgs {
    query: String,
    max_results: Option<usize>,
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    max_chars: Option<usize>,
}

fn is_skipped_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    SKIP_EXTENSIONS
        .iter()
        .any(|skipped| ext.eq_ignore_ascii_case(skipped))
}

fn clamp_usize(value: Option<usize>, default: usize, min: usize, max: usize) -> usize {
    value.unwrap_or(default).clamp(min, max)
}

fn validate_relative(path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(local_path(
                    path.display().to_string(),
                    "only relative paths without `..` are allowed",
                ));
            }
        }
    }
    Ok(())
}

fn ensure_within_root(root: &Path, canonical: &Path, requested: &Path) -> Result<(), Error> {
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err(local_path(
            requested.display().to_string(),
            "path escapes repository root",
        ))
    }
}

fn decode_args<T: for<'de> Deserialize<'de>>(tool: &str, value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|error| Error::InvalidToolArguments {
        tool: format!("local_tools.{tool}"),
        error: error.to_string(),
    })
}

fn invalid_args(tool: &str, error: impl Into<String>) -> Error {
    Error::InvalidToolArguments {
        tool: tool.to_string(),
        error: error.into(),
    }
}

fn local_path(path: String, error: impl Into<String>) -> Error {
    Error::LocalPath {
        path,
        error: error.into(),
    }
}

fn local_io(operation: &str, path: &Path, error: &std::io::Error) -> Error {
    Error::LocalIo {
        operation: operation.to_string(),
        path: path.display().to_string(),
        error: error.to_string(),
    }
}

fn display_relative_input(path: &Path) -> String {
    path.display().to_string().trim().to_string()
}

fn display_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    async fn fixture() -> (tempfile::TempDir, LocalTools) {
        let dir = tempfile::tempdir().expect("temp dir");
        let tools = LocalTools::new(dir.path()).await.expect("local tools");
        (dir, tools)
    }

    #[tokio::test]
    async fn search_docs_finds_text_and_skips_blacklisted_extensions() {
        let (dir, tools) = fixture().await;
        let root = dir.path();
        fs::create_dir_all(root.join("docs"))
            .await
            .expect("docs dir");
        fs::create_dir_all(root.join("assets"))
            .await
            .expect("assets dir");
        fs::create_dir_all(root.join("src"))
            .await
            .expect("src dir");
        fs::write(root.join("docs/readme.md"), "hello searchable phrase")
            .await
            .expect("write md");
        fs::write(root.join("assets/logo.png"), "searchable phrase")
            .await
            .expect("write png");
        fs::write(root.join("src/main.rs"), "fn main() { searchable phrase }")
            .await
            .expect("write rs");

        let result = tools
            .call(
                "search_docs",
                json!({"query": "searchable phrase", "max_results": 10}),
            )
            .await
            .expect("search");

        let files: HashSet<String> = result["matches"]
            .as_array()
            .expect("matches")
            .iter()
            .map(|entry| entry["file"].as_str().unwrap_or_default().to_string())
            .collect();

        assert!(files.contains("docs/readme.md"));
        assert!(files.contains("src/main.rs"));
        assert!(!files.contains("assets/logo.png"));
        assert_eq!(result["total_matches"], 2);
    }

    #[tokio::test]
    async fn search_docs_is_case_insensitive() {
        let (dir, tools) = fixture().await;
        fs::write(dir.path().join("note.txt"), "Hello SEARCHABLE Phrase")
            .await
            .expect("write");

        let result = tools
            .call("search_docs", json!({"query": "searchable phrase"}))
            .await
            .expect("search");

        assert_eq!(result["total_matches"], 1);
        assert_eq!(result["matches"][0]["file"], "note.txt");
        assert_eq!(result["matches"][0]["line"], 1);
    }

    #[test]
    fn contains_ignore_ascii_case_finds_needle() {
        assert!(contains_ignore_ascii_case("AbCdEf", "cde"));
        assert!(contains_ignore_ascii_case("AbCdEf", "CDE"));
        assert!(!contains_ignore_ascii_case("AbCdEf", "xyz"));
    }

    #[tokio::test]
    async fn search_docs_rejects_empty_query() {
        let (_dir, tools) = fixture().await;
        let error = tools
            .call("search_docs", json!({"query": "   "}))
            .await
            .expect_err("empty query");
        assert!(matches!(error, Error::InvalidToolArguments { .. }));
    }

    #[tokio::test]
    async fn read_file_returns_content_and_truncates() {
        let (dir, tools) = fixture().await;
        let root = dir.path();
        let content = "a".repeat(150);
        fs::write(root.join("note.txt"), &content)
            .await
            .expect("write");

        let read = tools
            .call("read_file", json!({"path": "note.txt"}))
            .await
            .expect("read");
        assert_eq!(read["path"], "note.txt");
        assert_eq!(read["content"], content);

        let truncated = tools
            .call("read_file", json!({"path": "note.txt", "max_chars": 100}))
            .await
            .expect("truncated read");
        assert_eq!(
            truncated["content"],
            format!("{}\n...[truncated]", "a".repeat(100))
        );
    }

    #[tokio::test]
    async fn read_file_rejects_parent_traversal() {
        let (_dir, tools) = fixture().await;
        let error = tools
            .call("read_file", json!({"path": "../outside.txt"}))
            .await
            .expect_err("traversal");
        assert!(matches!(error, Error::LocalPath { .. }));
    }
}
