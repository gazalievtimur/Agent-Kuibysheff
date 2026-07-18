use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::mcp::stdio_client::McpError;

const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "logs", ".cursor"];
const SKIP_EXTENSIONS: &[&str] = &[
    ".exe", ".dll", ".so", ".dylib", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf",
    ".zip", ".gz", ".7z", ".tar", ".rar", ".woff", ".woff2", ".ttf", ".otf", ".bin", ".lock",
    ".pyc", ".class", ".o", ".a", ".rlib",
];
const MAX_SEARCH_DEPTH: usize = 8;
const MAX_SEARCH_FILES: usize = 600;
const MAX_SEARCH_FILE_CHARS: usize = 250_000;
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
    /// Returns [`McpError`] if the directory cannot be resolved or is not a directory.
    pub async fn new(root: &Path) -> Result<Self, McpError> {
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
    /// Returns [`McpError`] for invalid arguments, paths, or I/O failures.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, McpError> {
        match tool {
            "search_docs" => {
                let args: SearchDocsArgs = decode_args(tool, arguments)?;
                self.search_docs(&args.query, args.max_results).await
            }
            "read_file" => {
                let args: ReadFileArgs = decode_args(tool, arguments)?;
                self.read_file(Path::new(&args.path), args.max_chars).await
            }
            _ => Err(McpError::UnknownTool {
                server: "local_tools".to_string(),
                tool: tool.to_string(),
            }),
        }
    }

    async fn search_docs(&self, query: &str, max_results: Option<usize>) -> Result<Value, McpError> {
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
        let lowered = query.to_lowercase();
        let files = self.collect_search_files(&self.root).await?;
        let mut matches = Vec::new();

        for file_path in files {
            if matches.len() >= max_results {
                break;
            }

            let Ok(text) = fs::read_to_string(&file_path).await else {
                continue;
            };
            if text.chars().count() > MAX_SEARCH_FILE_CHARS {
                continue;
            }

            for (idx, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&lowered) {
                    let relative = display_relative_path(&self.root, &file_path);
                    matches.push(json!({
                        "file": relative,
                        "line": idx + 1,
                        "snippet": line.trim().chars().take(MAX_SNIPPET_CHARS).collect::<String>(),
                    }));
                    break;
                }
            }
        }

        Ok(json!({
            "query": query,
            "total_matches": matches.len(),
            "matches": matches,
        }))
    }

    async fn read_file(&self, relative: &Path, max_chars: Option<usize>) -> Result<Value, McpError> {
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

    async fn collect_search_files(&self, root: &Path) -> Result<Vec<PathBuf>, McpError> {
        let mut files = Vec::new();
        let mut stack = vec![(root.to_path_buf(), 0usize)];

        while let Some((dir, depth)) = stack.pop() {
            if depth > MAX_SEARCH_DEPTH {
                continue;
            }

            let mut reader = fs::read_dir(&dir)
                .await
                .map_err(|error| local_io("read_dir", &dir, &error))?;

            while let Some(entry) = reader
                .next_entry()
                .await
                .map_err(|error| local_io("read_dir", &dir, &error))?
            {
                if files.len() >= MAX_SEARCH_FILES {
                    return Ok(files);
                }

                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|error| local_io("file_type", &path, &error))?;

                if file_type.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }

                if file_type.is_file() && !is_skipped_extension(&path) {
                    files.push(path);
                }
            }
        }

        Ok(files)
    }

    async fn resolve_inside_root(&self, relative: &Path) -> Result<PathBuf, McpError> {
        validate_relative(relative)?;
        let candidate = self.root.join(relative);
        let canonical = fs::canonicalize(&candidate)
            .await
            .map_err(|error| local_io("canonicalize", &candidate, &error))?;
        ensure_within_root(&self.root, &canonical, relative)?;
        Ok(canonical)
    }
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
    let normalized = format!(".{}", ext.to_lowercase());
    SKIP_EXTENSIONS.contains(&normalized.as_str())
}

fn clamp_usize(value: Option<usize>, default: usize, min: usize, max: usize) -> usize {
    value.unwrap_or(default).clamp(min, max)
}

fn validate_relative(path: &Path) -> Result<(), McpError> {
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

fn ensure_within_root(root: &Path, canonical: &Path, requested: &Path) -> Result<(), McpError> {
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err(local_path(
            requested.display().to_string(),
            "path escapes repository root",
        ))
    }
}

fn decode_args<T: for<'de> Deserialize<'de>>(tool: &str, value: Value) -> Result<T, McpError> {
    serde_json::from_value(value).map_err(|error| McpError::InvalidToolArguments {
        tool: format!("local_tools.{tool}"),
        error: error.to_string(),
    })
}

fn invalid_args(tool: &str, error: impl Into<String>) -> McpError {
    McpError::InvalidToolArguments {
        tool: tool.to_string(),
        error: error.into(),
    }
}

fn local_path(path: String, error: impl Into<String>) -> McpError {
    McpError::HomePath {
        path,
        error: error.into(),
    }
}

fn local_io(operation: &str, path: &Path, error: &std::io::Error) -> McpError {
    McpError::HomeIo {
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
    async fn search_docs_rejects_empty_query() {
        let (_dir, tools) = fixture().await;
        let error = tools
            .call("search_docs", json!({"query": "   "}))
            .await
            .expect_err("empty query");
        assert!(matches!(error, McpError::InvalidToolArguments { .. }));
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
        assert!(matches!(error, McpError::HomePath { .. }));
    }
}
