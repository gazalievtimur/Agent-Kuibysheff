use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;
use tokio::task;
use tracing::debug;

use crate::access::paths::{is_within_root, relative_components};
use crate::access::{PathOperation, WorkspaceFsPolicy};
use crate::tools::read_window::read_char_window;
use crate::tools::LocalToolsError;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "logs",
    ".cursor",
    "protected",
];
const SKIP_EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "dylib", "png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "zip", "gz",
    "7z", "tar", "rar", "woff", "woff2", "ttf", "otf", "bin", "lock", "pyc", "class", "o", "a",
    "rlib",
];
const DEFAULT_MAX_RESULTS: usize = 8;
const MIN_MAX_RESULTS: usize = 1;
const MAX_MAX_RESULTS: usize = 100;
const DEFAULT_READ_CHARS: usize = 6_000;
const MIN_READ_CHARS: usize = 100;
const MAX_READ_CHARS: usize = 200_000;
const MAX_SNIPPET_CHARS: usize = 300;

pub struct LocalTools {
    root: PathBuf,
    policy: WorkspaceFsPolicy,
}

impl LocalTools {
    /// Creates a repository-scoped toolset rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`LocalToolsError`] if the directory cannot be resolved or is not a directory.
    pub async fn new(root: &Path, policy: WorkspaceFsPolicy) -> Result<Self, LocalToolsError> {
        fs::create_dir_all(root)
            .await
            .map_err(|error| local_io("create_dir_all", root, error))?;
        let root = fs::canonicalize(root)
            .await
            .map_err(|error| local_io("canonicalize", root, error))?;
        if !fs::metadata(&root)
            .await
            .map_err(|error| local_io("metadata", &root, error))?
            .is_dir()
        {
            return Err(local_path(
                root.display().to_string(),
                "root is not a directory",
            ));
        }
        Ok(Self { root, policy })
    }

    /// Dispatches a local repository tool call.
    ///
    /// # Errors
    ///
    /// Returns [`LocalToolsError`] for invalid arguments, paths, or I/O failures.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, LocalToolsError> {
        match tool {
            "search_docs" => {
                let args: SearchDocsArgs = decode_args(tool, arguments)?;
                self.search_docs(&args.query, args.max_results).await
            }
            "read_file" => {
                let args: ReadFileArgs = decode_args(tool, arguments)?;
                self.read_file(Path::new(&args.path), args.offset, args.max_chars)
                    .await
            }
            _ => Err(LocalToolsError::UnknownTool {
                tool: tool.to_string(),
            }),
        }
    }

    async fn search_docs(
        &self,
        query: &str,
        max_results: Option<usize>,
    ) -> Result<Value, LocalToolsError> {
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
        let policy = self.policy.clone();
        let root_display = root.display().to_string();
        let query = query.to_owned();

        task::spawn_blocking(move || search_docs_blocking(&root, &policy, &query, max_results))
            .await
            .map_err(|error| LocalToolsError::Io {
                operation: "spawn_blocking".to_string(),
                path: root_display,
                source: std::io::Error::other(error.to_string()),
            })
    }

    async fn read_file(
        &self,
        relative: &Path,
        offset: Option<usize>,
        max_chars: Option<usize>,
    ) -> Result<Value, LocalToolsError> {
        let relative_display = display_relative_input(relative);
        if relative_display.is_empty() {
            return Err(invalid_args(
                "local_tools.read_file",
                "`path` must not be empty",
            ));
        }

        self.policy
            .read
            .allows_relative(relative, PathOperation::Read)
            .map_err(|reason| local_path(relative.display().to_string(), reason))?;

        let path = self.resolve_existing_file(relative).await?;
        let displayed = display_relative_path(&self.root, &path);
        let offset = offset.unwrap_or(0);
        let max_chars = clamp_usize(
            max_chars,
            DEFAULT_READ_CHARS,
            MIN_READ_CHARS,
            MAX_READ_CHARS,
        );
        let path_display = path.display().to_string();
        let window = task::spawn_blocking(move || read_char_window(&path, offset, max_chars))
            .await
            .map_err(|error| LocalToolsError::Io {
                operation: "spawn_blocking".to_string(),
                path: path_display.clone(),
                source: std::io::Error::other(error.to_string()),
            })?
            .map_err(|error| local_io("read_char_window", Path::new(&path_display), error))?;

        Ok(json!({
            "path": displayed,
            "content": window.content,
            "offset": window.offset,
            "chars_returned": window.chars_returned,
            "truncated": window.truncated,
            "next_offset": window.next_offset,
        }))
    }

    async fn resolve_existing_file(&self, relative: &Path) -> Result<PathBuf, LocalToolsError> {
        relative_components(relative)
            .map_err(|reason| local_path(relative.display().to_string(), reason))?;
        let candidate = self.root.join(relative);
        let symlink_meta = fs::symlink_metadata(&candidate)
            .await
            .map_err(|error| local_io("symlink_metadata", &candidate, error))?;
        if symlink_meta.file_type().is_symlink() {
            let canonical = fs::canonicalize(&candidate)
                .await
                .map_err(|error| local_io("canonicalize", &candidate, error))?;
            ensure_within_root(&self.root, &canonical, relative)?;
            ensure_grant_for_canonical(&self.root, &self.policy, &canonical, relative)?;
            if !fs::metadata(&canonical)
                .await
                .map_err(|error| local_io("metadata", &canonical, error))?
                .is_file()
            {
                return Err(local_path(
                    relative.display().to_string(),
                    "symlink target is not a file",
                ));
            }
            return Ok(canonical);
        }

        let canonical = fs::canonicalize(&candidate)
            .await
            .map_err(|error| local_io("canonicalize", &candidate, error))?;
        ensure_within_root(&self.root, &canonical, relative)?;
        ensure_grant_for_canonical(&self.root, &self.policy, &canonical, relative)?;
        if !fs::metadata(&canonical)
            .await
            .map_err(|error| local_io("metadata", &canonical, error))?
            .is_file()
        {
            return Err(local_path(
                relative.display().to_string(),
                "path is not a file",
            ));
        }
        Ok(canonical)
    }
}

fn search_docs_blocking(
    root: &Path,
    policy: &WorkspaceFsPolicy,
    query: &str,
    max_results: usize,
) -> Value {
    let query_lower = query.to_lowercase();
    let ascii_needle = query_lower.is_ascii();
    let mut matches = Vec::with_capacity(max_results.min(MAX_MAX_RESULTS));
    let mut stack = initial_search_dirs(root, policy);

    while let Some(dir) = stack.pop() {
        if matches.len() >= max_results {
            break;
        }

        if !directory_allowed(root, policy, &dir) {
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
            if matches.len() >= max_results {
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

            if file_type.is_symlink() {
                if let Some(found) =
                    match_symlink_file(root, policy, &path, &query_lower, ascii_needle)
                {
                    matches.push(found);
                }
                continue;
            }

            if file_type.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !SKIP_DIRS.contains(&name.as_ref()) && directory_allowed(root, policy, &path) {
                    stack.push(path);
                }
                continue;
            }

            if !file_type.is_file() || is_skipped_extension(&path) {
                continue;
            }

            if !file_allowed(root, policy, &path) {
                continue;
            }
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

fn initial_search_dirs(root: &Path, policy: &WorkspaceFsPolicy) -> Vec<PathBuf> {
    if policy.read.is_legacy() {
        return vec![root.to_path_buf()];
    }
    if policy.read.grants().is_empty() {
        return Vec::new();
    }

    let mut dirs = Vec::with_capacity(policy.read.grants().len());
    for grant in policy.read.grants() {
        let path = if grant.components().is_empty() {
            root.to_path_buf()
        } else {
            root.join(grant.as_path())
        };
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs
}

fn directory_allowed(root: &Path, policy: &WorkspaceFsPolicy, dir: &Path) -> bool {
    let Ok(canonical) = std::fs::canonicalize(dir) else {
        return false;
    };
    if !is_within_root(root, &canonical) {
        return false;
    }
    let Ok(relative) = canonical.strip_prefix(root) else {
        return false;
    };
    policy
        .read
        .allows_relative(relative, PathOperation::Read)
        .is_ok()
}

fn file_allowed(root: &Path, policy: &WorkspaceFsPolicy, path: &Path) -> bool {
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return false;
    };
    ensure_within_root(root, &canonical, path).is_ok()
        && ensure_grant_for_canonical(root, policy, &canonical, path).is_ok()
}

fn match_symlink_file(
    root: &Path,
    policy: &WorkspaceFsPolicy,
    path: &Path,
    query_lower: &str,
    ascii_needle: bool,
) -> Option<Value> {
    let canonical = std::fs::canonicalize(path).ok()?;
    ensure_within_root(root, &canonical, path).ok()?;
    ensure_grant_for_canonical(root, policy, &canonical, path).ok()?;
    let metadata = std::fs::metadata(&canonical).ok()?;
    if !metadata.is_file() || is_skipped_extension(&canonical) {
        return None;
    }
    match_file(root, &canonical, query_lower, ascii_needle)
}

fn match_file(root: &Path, path: &Path, query_lower: &str, ascii_needle: bool) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let reader = io::BufReader::new(file);
    for (idx, line) in reader.lines().enumerate() {
        let Ok(line) = line else {
            continue;
        };
        if line_matches(&line, query_lower, ascii_needle) {
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
    offset: Option<usize>,
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

fn ensure_within_root(
    root: &Path,
    canonical: &Path,
    requested: &Path,
) -> Result<(), LocalToolsError> {
    if crate::access::is_denied_protected_path(None, canonical) {
        return Err(local_path(
            requested.display().to_string(),
            crate::access::PROTECTED_DENY_REASON,
        ));
    }
    if is_within_root(root, canonical) {
        Ok(())
    } else {
        Err(local_path(
            requested.display().to_string(),
            "path escapes repository root",
        ))
    }
}

fn ensure_grant_for_canonical(
    root: &Path,
    policy: &WorkspaceFsPolicy,
    canonical: &Path,
    requested: &Path,
) -> Result<(), LocalToolsError> {
    let relative = canonical.strip_prefix(root).map_err(|_| {
        local_path(
            requested.display().to_string(),
            "path escapes repository root",
        )
    })?;
    policy
        .read
        .allows_relative(relative, PathOperation::Read)
        .map_err(|reason| local_path(requested.display().to_string(), reason))
}

fn decode_args<T: for<'de> Deserialize<'de>>(
    tool: &str,
    value: Value,
) -> Result<T, LocalToolsError> {
    serde_json::from_value(value).map_err(|error| LocalToolsError::InvalidArguments {
        error: format!("local_tools.{tool}: {error}"),
    })
}

fn invalid_args(tool: &str, error: impl Into<String>) -> LocalToolsError {
    LocalToolsError::InvalidArguments {
        error: format!("{tool}: {}", error.into()),
    }
}

fn local_path(path: String, error: impl Into<String>) -> LocalToolsError {
    LocalToolsError::PathDenied {
        path,
        error: error.into(),
    }
}

fn local_io(operation: &str, path: &Path, error: std::io::Error) -> LocalToolsError {
    LocalToolsError::Io {
        operation: operation.to_string(),
        path: path.display().to_string(),
        source: error,
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
    use crate::access::paths::PathGrantScope;
    use crate::access::RelativeGrant;

    async fn fixture() -> (tempfile::TempDir, LocalTools) {
        let dir = tempfile::tempdir().expect("temp dir");
        let tools = LocalTools::new(dir.path(), WorkspaceFsPolicy::legacy())
            .await
            .expect("local tools");
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
        fs::create_dir_all(root.join("src")).await.expect("src dir");
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
        assert!(matches!(error, LocalToolsError::InvalidArguments { .. }));
    }

    #[tokio::test]
    async fn read_file_returns_content_and_windows() {
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
        assert_eq!(read["truncated"], false);
        assert!(read["next_offset"].is_null());

        let truncated = tools
            .call("read_file", json!({"path": "note.txt", "max_chars": 100}))
            .await
            .expect("truncated read");
        assert_eq!(truncated["content"], "a".repeat(100));
        assert_eq!(truncated["truncated"], true);
        assert_eq!(truncated["chars_returned"], 100);
        assert_eq!(truncated["next_offset"], 100);

        let second = tools
            .call(
                "read_file",
                json!({"path": "note.txt", "offset": 100, "max_chars": 100}),
            )
            .await
            .expect("second window");
        assert_eq!(second["content"], "a".repeat(50));
        assert_eq!(second["truncated"], false);
        assert!(second["next_offset"].is_null());
    }

    #[tokio::test]
    async fn read_file_allows_oversized_files_via_windows() {
        let (dir, tools) = fixture().await;
        // Larger than the old 800 KB hard reject (MAX_READ_CHARS * 4).
        let content = "x".repeat(900_000);
        fs::write(dir.path().join("huge.txt"), &content)
            .await
            .expect("write huge");

        let first = tools
            .call("read_file", json!({"path": "huge.txt", "max_chars": 1000}))
            .await
            .expect("first window");
        assert_eq!(first["content"].as_str().unwrap().len(), 1000);
        assert_eq!(first["truncated"], true);
        assert_eq!(first["next_offset"], 1000);

        let mid = tools
            .call(
                "read_file",
                json!({"path": "huge.txt", "offset": 899_500, "max_chars": 1000}),
            )
            .await
            .expect("tail window");
        assert_eq!(mid["content"], "x".repeat(500));
        assert_eq!(mid["truncated"], false);
    }

    #[tokio::test]
    async fn search_docs_finds_needle_in_large_file() {
        let (dir, tools) = fixture().await;
        let mut body = "noise line\n".repeat(30_000); // ~330 KB
        body.push_str("unique_needle_token_xyz\n");
        body.push_str(&"more noise\n".repeat(100));
        fs::write(dir.path().join("big.log"), body)
            .await
            .expect("write big");

        let result = tools
            .call("search_docs", json!({"query": "unique_needle_token_xyz"}))
            .await
            .expect("search");
        assert_eq!(result["total_matches"], 1);
        assert_eq!(result["matches"][0]["file"], "big.log");
    }

    #[tokio::test]
    async fn search_docs_finds_needle_beyond_old_file_cap() {
        let (dir, tools) = fixture().await;
        let root = dir.path();
        // More than the old MAX_SEARCH_FILES=600 silent cap.
        for i in 0..650 {
            let bucket = format!("bucket-{:02}", i % 20);
            let path = root.join(&bucket).join(format!("doc-{i:04}.txt"));
            fs::create_dir_all(path.parent().unwrap())
                .await
                .expect("dir");
            let text = if i == 640 {
                "marker FINDME_DEEP"
            } else {
                "ordinary noise"
            };
            fs::write(&path, text).await.expect("write");
        }

        let result = tools
            .call("search_docs", json!({"query": "FINDME_DEEP"}))
            .await
            .expect("search");
        assert_eq!(result["total_matches"], 1);
        assert!(result["matches"][0]["file"]
            .as_str()
            .unwrap()
            .contains("doc-0640.txt"));
    }

    #[tokio::test]
    async fn read_file_rejects_parent_traversal() {
        let (_dir, tools) = fixture().await;
        let error = tools
            .call("read_file", json!({"path": "../outside.txt"}))
            .await
            .expect_err("traversal");
        assert!(matches!(error, LocalToolsError::PathDenied { .. }));
    }

    #[tokio::test]
    async fn read_file_respects_strict_grants() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("src"))
            .await
            .expect("src");
        fs::create_dir_all(dir.path().join("secret"))
            .await
            .expect("secret");
        fs::write(dir.path().join("src/ok.txt"), "ok")
            .await
            .expect("ok");
        fs::write(dir.path().join("secret/no.txt"), "no")
            .await
            .expect("no");

        let policy = WorkspaceFsPolicy {
            read: PathGrantScope::strict(vec![RelativeGrant::parse("src").unwrap()]),
        };
        let tools = LocalTools::new(dir.path(), policy).await.expect("tools");

        tools
            .call("read_file", json!({"path": "src/ok.txt"}))
            .await
            .expect("allowed");
        let err = tools
            .call("read_file", json!({"path": "secret/no.txt"}))
            .await
            .expect_err("denied");
        assert!(matches!(err, LocalToolsError::PathDenied { .. }));
    }

    #[tokio::test]
    async fn search_docs_respects_strict_grants() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("docs"))
            .await
            .expect("docs");
        fs::create_dir_all(dir.path().join("other"))
            .await
            .expect("other");
        fs::write(dir.path().join("docs/a.txt"), "needle here")
            .await
            .expect("docs");
        fs::write(dir.path().join("other/b.txt"), "needle here")
            .await
            .expect("other");

        let policy = WorkspaceFsPolicy {
            read: PathGrantScope::strict(vec![RelativeGrant::parse("docs").unwrap()]),
        };
        let tools = LocalTools::new(dir.path(), policy).await.expect("tools");
        let result = tools
            .call("search_docs", json!({"query": "needle"}))
            .await
            .expect("search");
        assert_eq!(result["total_matches"], 1);
        assert_eq!(result["matches"][0]["file"], "docs/a.txt");
    }
}
