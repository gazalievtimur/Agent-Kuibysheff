//! Read-only access to skill folders indexed at startup.
//!
//! Arguments are a skill name and an optional relative path. The path is accepted
//! only when it is already in the index. Reads never accept a filesystem path into
//! `protected/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::skills::package::{SkillPackage, SKILL_BODY_FILE};
use crate::tool_api::ProfileSkillError;

use super::read_window::read_char_window;

const DEFAULT_MAX_CHARS: usize = 50_000;
const MAX_READ_CHARS: usize = 200_000;

/// Snapshot of skill packages for one run.
#[derive(Debug, Clone, Default)]
pub struct SkillLibrary {
    packages: BTreeMap<String, SkillPackage>,
}

impl SkillLibrary {
    #[must_use]
    pub fn new(packages: BTreeMap<String, SkillPackage>) -> Self {
        Self { packages }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Dispatches `list_skill` and `read_skill`.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileSkillError`] for an unknown skill, a path outside the index,
    /// invalid arguments, or a non-UTF-8 file.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, ProfileSkillError> {
        match tool {
            "list_skill" => {
                let args: NameArgs = decode(tool, arguments)?;
                self.list(&args.name)
            }
            "read_skill" => {
                let args: ReadArgs = decode(tool, arguments)?;
                self.read(
                    &args.name,
                    args.path.as_deref(),
                    args.offset,
                    args.max_chars,
                )
                .await
            }
            _ => Err(ProfileSkillError::UnknownTool {
                tool: tool.to_string(),
            }),
        }
    }

    fn list(&self, name: &str) -> Result<Value, ProfileSkillError> {
        let package = self.package(name)?;
        let files: Vec<&str> = package
            .files
            .iter()
            .map(|file| file.relative.as_str())
            .collect();
        Ok(json!({
            "name": name,
            "files": files,
        }))
    }

    async fn read(
        &self,
        name: &str,
        path: Option<&str>,
        offset: Option<usize>,
        max_chars: Option<usize>,
    ) -> Result<Value, ProfileSkillError> {
        let package = self.package(name)?;
        let relative = path.unwrap_or(SKILL_BODY_FILE);
        let entry = package
            .files
            .iter()
            .find(|file| file.relative == relative)
            .ok_or_else(|| ProfileSkillError::Denied {
                error: format!("`{relative}` is not in skill `{name}`"),
            })?;
        let offset = offset.unwrap_or(0);
        let max_chars = max_chars
            .unwrap_or(DEFAULT_MAX_CHARS)
            .clamp(1, MAX_READ_CHARS);
        let canonical = entry.canonical.clone();
        let root = package.root.clone();
        let relative_owned = entry.relative.clone();
        let window = tokio::task::spawn_blocking(move || {
            ensure_still_inside(&root, &canonical)?;
            read_char_window(&canonical, offset, max_chars)
        })
        .await
        .map_err(|error| ProfileSkillError::Io {
            operation: "read_skill".to_string(),
            path: relative_owned.clone(),
            source: std::io::Error::other(error.to_string()),
        })?
        .map_err(|error| ProfileSkillError::Io {
            operation: "read_skill".to_string(),
            path: relative_owned.clone(),
            source: error,
        })?;
        Ok(json!({
            "name": name,
            "path": relative_owned,
            "content": window.content,
            "offset": window.offset,
            "chars_returned": window.chars_returned,
            "truncated": window.truncated,
            "next_offset": window.next_offset,
        }))
    }

    fn package(&self, name: &str) -> Result<&SkillPackage, ProfileSkillError> {
        self.packages
            .get(name)
            .ok_or_else(|| ProfileSkillError::Denied {
                error: format!("skill `{name}` has no folder"),
            })
    }
}

fn ensure_still_inside(root: &Path, path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "skill file must not be a symlink",
        ));
    }
    let canonical = path.canonicalize()?;
    let root = root.canonicalize()?;
    if !canonical.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "skill file escapes the skill directory",
        ));
    }
    Ok(())
}

fn decode<T: for<'de> Deserialize<'de>>(
    tool: &str,
    arguments: Value,
) -> Result<T, ProfileSkillError> {
    serde_json::from_value(arguments).map_err(|error| ProfileSkillError::InvalidArguments {
        error: format!("{tool}: {error}"),
    })
}

#[derive(Debug, Deserialize)]
struct NameArgs {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    name: String,
    path: Option<String>,
    offset: Option<usize>,
    max_chars: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::package::{self, SKILL_PACKAGES_DIR};
    use std::fs;

    #[tokio::test]
    async fn lists_and_reads_skill_md_and_script_window() {
        let dir = tempfile::tempdir().expect("temp");
        let skill = dir.path().join(SKILL_PACKAGES_DIR).join("research");
        fs::create_dir_all(skill.join("scripts")).expect("dirs");
        fs::write(skill.join("skill.md"), "# Research\n").expect("body");
        let script = "x".repeat(80);
        fs::write(skill.join("scripts").join("example.py"), &script).expect("script");
        let packages = package::load_skill_packages(dir.path()).expect("index");
        let library = SkillLibrary::new(packages);

        let listed = library
            .call("list_skill", json!({"name": "research"}))
            .await
            .expect("list");
        assert_eq!(listed["files"], json!(["scripts/example.py", "skill.md"]));

        let body = library
            .call("read_skill", json!({"name": "research"}))
            .await
            .expect("body");
        assert_eq!(body["path"], "skill.md");
        assert_eq!(body["content"], "# Research\n");

        let window = library
            .call(
                "read_skill",
                json!({"name": "research", "path": "scripts/example.py", "max_chars": 10}),
            )
            .await
            .expect("window");
        assert_eq!(window["content"], "xxxxxxxxxx");
        assert_eq!(window["truncated"], true);
        assert_eq!(window["next_offset"], 10);

        let missing = library
            .call(
                "read_skill",
                json!({"name": "research", "path": "../secret"}),
            )
            .await
            .expect_err("escape");
        assert!(missing.to_string().contains("not in skill"), "{missing}");

        let unknown = library
            .call("read_skill", json!({"name": "other"}))
            .await
            .expect_err("unknown");
        assert!(unknown.to_string().contains("no folder"), "{unknown}");
        assert!(!library.is_empty());
        assert!(SkillLibrary::default().is_empty());

        let bad_tool = library
            .call("nope", json!({}))
            .await
            .expect_err("unknown tool");
        assert!(bad_tool.to_string().contains("unknown"), "{bad_tool}");
        let bad_args = library
            .call("list_skill", json!({}))
            .await
            .expect_err("args");
        assert!(bad_args.to_string().contains("invalid"), "{bad_args}");

        let clamped = library
            .call(
                "read_skill",
                json!({"name": "research", "offset": 2, "max_chars": 9_999_999}),
            )
            .await
            .expect("clamp");
        assert_eq!(clamped["offset"], 2);
        assert_eq!(clamped["truncated"], false);

        fs::write(skill.join("scripts").join("blob.bin"), [0xff, 0xfe]).expect("bin");
        let packages = package::load_skill_packages(dir.path()).expect("reindex");
        let library = SkillLibrary::new(packages);
        let binary = library
            .call(
                "read_skill",
                json!({"name": "research", "path": "scripts/blob.bin"}),
            )
            .await
            .expect_err("utf8");
        assert!(
            binary.to_string().contains("utf-8") || binary.to_string().contains("read_skill"),
            "{binary}"
        );
    }
}
