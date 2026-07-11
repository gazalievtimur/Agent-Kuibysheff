use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::mcp::stdio_client::McpError;

const DEFAULT_MAX_CHARS: usize = 50_000;
const MAX_READ_CHARS: usize = 200_000;

pub struct HomeFs {
    root: PathBuf,
}

impl HomeFs {
    /// Creates a sandboxed filesystem rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the directory cannot be created or is not a directory.
    pub async fn new(root: &Path) -> Result<Self, McpError> {
        fs::create_dir_all(root)
            .await
            .map_err(|error| home_io("create_dir_all", root, &error))?;
        let root = fs::canonicalize(root)
            .await
            .map_err(|error| home_io("canonicalize", root, &error))?;
        if !fs::metadata(&root)
            .await
            .map_err(|error| home_io("metadata", &root, &error))?
            .is_dir()
        {
            return Err(McpError::HomePath {
                path: root.display().to_string(),
                error: "home is not a directory".to_string(),
            });
        }
        Ok(Self { root })
    }

    /// Dispatches a home filesystem tool call.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] for invalid arguments, paths, or I/O failures.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, McpError> {
        match tool {
            "list" => {
                let args: ListArgs = decode_args(tool, arguments)?;
                self.list(Path::new(&args.path)).await
            }
            "read" => {
                let args: ReadArgs = decode_args(tool, arguments)?;
                self.read(Path::new(&args.path), args.max_chars).await
            }
            "write" => {
                let args: WriteArgs = decode_args(tool, arguments)?;
                self.write(Path::new(&args.path), &args.content).await
            }
            _ => Err(McpError::UnknownTool {
                server: "home".to_string(),
                tool: tool.to_string(),
            }),
        }
    }

    async fn list(&self, relative: &Path) -> Result<Value, McpError> {
        let path = self.resolve_existing(relative).await?;
        let metadata = fs::metadata(&path)
            .await
            .map_err(|error| home_io("metadata", &path, &error))?;
        if !metadata.is_dir() {
            return Err(McpError::HomePath {
                path: relative.display().to_string(),
                error: "path is not a directory".to_string(),
            });
        }

        let mut reader = fs::read_dir(&path)
            .await
            .map_err(|error| home_io("read_dir", &path, &error))?;
        let mut entries = Vec::new();
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|error| home_io("read_dir", &path, &error))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| home_io("file_type", &entry.path(), &error))?;
            entries.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "kind": if file_type.is_dir() {
                    "directory"
                } else if file_type.is_file() {
                    "file"
                } else if file_type.is_symlink() {
                    "symlink"
                } else {
                    "other"
                }
            }));
        }
        entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["name"].as_str().unwrap_or_default())
        });

        Ok(json!({
            "path": display_relative(relative),
            "entries": entries
        }))
    }

    async fn read(&self, relative: &Path, max_chars: Option<usize>) -> Result<Value, McpError> {
        let path = self.resolve_existing(relative).await?;
        let content = fs::read_to_string(&path)
            .await
            .map_err(|error| home_io("read_to_string", &path, &error))?;
        let max_chars = max_chars
            .unwrap_or(DEFAULT_MAX_CHARS)
            .clamp(1, MAX_READ_CHARS);
        let total_chars = content.chars().count();
        let truncated = total_chars > max_chars;
        let content = if truncated {
            content.chars().take(max_chars).collect::<String>()
        } else {
            content
        };

        Ok(json!({
            "path": display_relative(relative),
            "content": content,
            "truncated": truncated
        }))
    }

    async fn write(&self, relative: &Path, content: &str) -> Result<Value, McpError> {
        validate_relative(relative)?;
        let file_name = relative
            .file_name()
            .ok_or_else(|| invalid_path(relative, "path must name a file"))?;
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = self.ensure_directories(parent).await?;
        let destination = parent.join(file_name);

        if fs::symlink_metadata(&destination).await.is_ok() {
            let canonical = fs::canonicalize(&destination)
                .await
                .map_err(|error| home_io("canonicalize", &destination, &error))?;
            self.ensure_within_home(&canonical, relative)?;
            if fs::metadata(&canonical)
                .await
                .map_err(|error| home_io("metadata", &canonical, &error))?
                .is_dir()
            {
                return Err(invalid_path(relative, "path is a directory"));
            }
        }

        fs::write(&destination, content)
            .await
            .map_err(|error| home_io("write", &destination, &error))?;
        Ok(json!({
            "path": display_relative(relative),
            "bytes_written": content.len()
        }))
    }

    async fn resolve_existing(&self, relative: &Path) -> Result<PathBuf, McpError> {
        validate_relative(relative)?;
        let candidate = self.root.join(relative);
        let canonical = fs::canonicalize(&candidate)
            .await
            .map_err(|error| home_io("canonicalize", &candidate, &error))?;
        self.ensure_within_home(&canonical, relative)?;
        Ok(canonical)
    }

    async fn ensure_directories(&self, relative: &Path) -> Result<PathBuf, McpError> {
        validate_relative(relative)?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                continue;
            };
            let next = current.join(part);
            match fs::symlink_metadata(&next).await {
                Ok(_) => {
                    let canonical = fs::canonicalize(&next)
                        .await
                        .map_err(|error| home_io("canonicalize", &next, &error))?;
                    self.ensure_within_home(&canonical, relative)?;
                    if !fs::metadata(&canonical)
                        .await
                        .map_err(|error| home_io("metadata", &canonical, &error))?
                        .is_dir()
                    {
                        return Err(invalid_path(
                            relative,
                            "parent component is not a directory",
                        ));
                    }
                    current = canonical;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&next)
                        .await
                        .map_err(|error| home_io("create_dir", &next, &error))?;
                    current = fs::canonicalize(&next)
                        .await
                        .map_err(|error| home_io("canonicalize", &next, &error))?;
                    self.ensure_within_home(&current, relative)?;
                }
                Err(error) => return Err(home_io("symlink_metadata", &next, &error)),
            }
        }
        Ok(current)
    }

    fn ensure_within_home(&self, canonical: &Path, requested: &Path) -> Result<(), McpError> {
        if canonical.starts_with(&self.root) {
            Ok(())
        } else {
            Err(invalid_path(requested, "path resolves outside home"))
        }
    }
}

#[derive(Deserialize)]
struct ListArgs {
    #[serde(default = "default_list_path")]
    path: String,
}

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    max_chars: Option<usize>,
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

fn default_list_path() -> String {
    ".".to_string()
}

fn decode_args<T: for<'de> Deserialize<'de>>(tool: &str, value: Value) -> Result<T, McpError> {
    serde_json::from_value(value).map_err(|error| McpError::InvalidToolArguments {
        tool: format!("home.{tool}"),
        error: error.to_string(),
    })
}

fn validate_relative(path: &Path) -> Result<(), McpError> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_path(
                    path,
                    "only relative paths without `..` are allowed",
                ));
            }
        }
    }
    Ok(())
}

fn invalid_path(path: &Path, error: impl Into<String>) -> McpError {
    McpError::HomePath {
        path: path.display().to_string(),
        error: error.into(),
    }
}

fn home_io(operation: &str, path: &Path, error: &std::io::Error) -> McpError {
    McpError::HomeIo {
        operation: operation.to_string(),
        path: path.display().to_string(),
        error: error.to_string(),
    }
}

fn display_relative(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        ".".to_string()
    } else {
        path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_reads_and_lists_inside_home() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = HomeFs::new(dir.path()).await.expect("home");

        home.call(
            "write",
            json!({"path": "out/src/main.rs", "content": "fn main() {}"}),
        )
        .await
        .expect("write");
        let read = home
            .call("read", json!({"path": "out/src/main.rs"}))
            .await
            .expect("read");
        assert_eq!(read["content"], "fn main() {}");

        let list = home
            .call("list", json!({"path": "out/src"}))
            .await
            .expect("list");
        assert_eq!(list["entries"][0]["name"], "main.rs");
    }

    #[tokio::test]
    async fn rejects_parent_traversal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = HomeFs::new(dir.path()).await.expect("home");

        let error = home
            .call("write", json!({"path": "../outside.txt", "content": "no"}))
            .await
            .expect_err("must reject traversal");
        assert!(matches!(error, McpError::HomePath { .. }));
    }
}
