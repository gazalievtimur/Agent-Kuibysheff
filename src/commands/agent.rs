//! `kbshff agent list` and `kbshff agent out`.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use thiserror::Error;

use crate::cli::{AgentArgs, AgentCommand, AgentOutCommand, ConfigFormat};
use crate::project_paths::{list_agent_ids, resolve_agent_identity, AgentPathError};

const MAX_OUT_PREVIEW_BYTES: u64 = 512 * 1024;

/// Errors from `agent` operator commands.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentCmdError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    AgentPath(#[from] AgentPathError),
    #[error("{0}")]
    Message(String),
}

/// Run `kbshff agent`.
///
/// # Errors
///
/// Returns I/O, path, or preview failures.
pub fn run(args: &AgentArgs) -> Result<(), AgentCmdError> {
    match &args.command {
        AgentCommand::List(list) => {
            let agents = list_agent_ids(&list.project_root)?;
            match list.format {
                ConfigFormat::Json => print_json(&json!({ "agents": agents })),
                ConfigFormat::Text => {
                    let mut out = io::stdout().lock();
                    for id in agents {
                        writeln!(out, "{id}")?;
                    }
                    Ok(())
                }
            }
        }
        AgentCommand::Out(out_args) => {
            let paths = resolve_agent_identity(
                &out_args.identity.project_root,
                &out_args.identity.agent,
                None,
            )?;
            let out_dir = paths.home.join("out");
            match &out_args.command {
                AgentOutCommand::List => {
                    let files = list_out_files(&out_dir)?;
                    match out_args.format {
                        ConfigFormat::Json => print_json(&json!({ "files": files })),
                        ConfigFormat::Text => {
                            let mut out = io::stdout().lock();
                            for file in files {
                                let name = file.get("name").and_then(Value::as_str).unwrap_or("");
                                let size = file.get("size").and_then(Value::as_u64).unwrap_or(0);
                                writeln!(out, "{name}\t{size}")?;
                            }
                            Ok(())
                        }
                    }
                }
                AgentOutCommand::Show { name } => {
                    let file = safe_out_file(&out_dir, name).map_err(AgentCmdError::Message)?;
                    let content =
                        read_tail(&file, MAX_OUT_PREVIEW_BYTES).map_err(AgentCmdError::Message)?;
                    match out_args.format {
                        ConfigFormat::Json => {
                            print_json(&json!({ "name": name, "content": content }))
                        }
                        ConfigFormat::Text => {
                            let mut out = io::stdout().lock();
                            write!(out, "{content}")?;
                            Ok(())
                        }
                    }
                }
            }
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<(), AgentCmdError> {
    crate::commands::print_json(value).map_err(AgentCmdError::from)
}

fn list_out_files(out_dir: &Path) -> Result<Vec<Value>, AgentCmdError> {
    if !out_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(out_dir)? {
        let entry = entry?;
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains(['/', '\\']) || name == ".." || name == "." {
            continue;
        }
        files.push(json!({
            "name": name,
            "size": meta.len(),
        }));
    }
    files.sort_by(|a, b| {
        a.get("name")
            .and_then(Value::as_str)
            .cmp(&b.get("name").and_then(Value::as_str))
    });
    Ok(files)
}

fn safe_out_file(out_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('\0') {
        return Err("invalid file name".to_string());
    }
    let candidate = Path::new(name);
    if candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || candidate.components().count() != 1
    {
        return Err("file name must be a single path segment under out/".to_string());
    }
    let full = out_dir.join(name);
    let out_canon = out_dir
        .canonicalize()
        .map_err(|err| format!("out dir: {err}"))?;
    let full_canon = full.canonicalize().map_err(|err| format!("file: {err}"))?;
    if !full_canon.starts_with(&out_canon) {
        return Err("path escapes out/".to_string());
    }
    if !full_canon.is_file() {
        return Err("not a file".to_string());
    }
    Ok(full_canon)
}

fn read_tail(path: &Path, max_bytes: u64) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|err| err.to_string())?;
    let mut file = File::open(path).map_err(|err| err.to_string())?;
    if meta.len() > max_bytes {
        file.seek(SeekFrom::End(-(max_bytes as i64)))
            .map_err(|err| err.to_string())?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|err| err.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_show_rejects_parent_and_reads_one_segment() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let out_dir = tmp.path().join("out");
        fs::create_dir_all(&out_dir).expect("out dir");
        fs::write(out_dir.join("note.txt"), "hello").expect("write");
        assert!(safe_out_file(&out_dir, "..").is_err());
        assert!(safe_out_file(&out_dir, "a/b").is_err());
        let path = safe_out_file(&out_dir, "note.txt").expect("file");
        let text = read_tail(&path, 32).expect("tail");
        assert_eq!(text, "hello");
    }
}
