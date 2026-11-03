//! Subprocess client for the `kbshff` CLI.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// How to find and invoke `kbshff`.
#[derive(Debug, Clone)]
pub struct Kbshff {
    pub bin: PathBuf,
    pub project_root: PathBuf,
}

/// Captured CLI result.
#[derive(Debug)]
pub struct ExecResult {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Kbshff {
    /// Run `kbshff` to completion.
    ///
    /// # Errors
    ///
    /// Returns when the binary cannot be spawned or the pipes fail.
    pub async fn exec(
        &self,
        args: &[String],
        stdin: Option<Vec<u8>>,
    ) -> Result<ExecResult, String> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|err| {
            format!(
                "failed to start `{}`: {err}. Install kbshff or pass --kbshff / set KBSHFF_BIN.",
                self.bin.display()
            )
        })?;
        if let Some(bytes) = stdin {
            if let Some(mut pipe) = child.stdin.take() {
                pipe.write_all(&bytes)
                    .await
                    .map_err(|err| format!("write kbshff stdin: {err}"))?;
            }
        }
        let output = child
            .wait_with_output()
            .await
            .map_err(|err| format!("wait for kbshff: {err}"))?;
        Ok(ExecResult {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Parse stdout JSON. A JSON object is returned even when the process exits non-zero
    /// (for example `check` with failed probes). `{ "ok": false }` is an error.
    ///
    /// # Errors
    ///
    /// Returns the CLI error text when stdout is not a successful JSON payload.
    pub async fn exec_json(
        &self,
        args: &[String],
        stdin: Option<Vec<u8>>,
    ) -> Result<Value, String> {
        let result = self.exec(args, stdin).await?;
        stdout_json(&result)
    }

    #[must_use]
    pub fn config_prefix(&self, agent: &str) -> Vec<String> {
        vec![
            "config".to_string(),
            "--project-root".to_string(),
            self.project_root.display().to_string(),
            "--agent".to_string(),
            agent.to_string(),
            "--format".to_string(),
            "json".to_string(),
        ]
    }

    #[must_use]
    pub fn identity_prefix(&self, command: &str, agent: &str) -> Vec<String> {
        vec![
            command.to_string(),
            "--project-root".to_string(),
            self.project_root.display().to_string(),
            "--agent".to_string(),
            agent.to_string(),
            "--format".to_string(),
            "json".to_string(),
        ]
    }
}

/// Resolve the `kbshff` binary: flag, `KBSHFF_BIN`, sibling of this exe, then `PATH`.
///
/// # Errors
///
/// Returns when an explicit path was requested but cannot be described. A missing
/// `PATH` entry is left as a bare name so spawn reports it.
pub fn resolve_kbshff(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("KBSHFF_BIN") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let name = bin_name("kbshff");
            let sibling = dir.join(name);
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }
    Ok(PathBuf::from(bin_name("kbshff")))
}

#[must_use]
pub fn bin_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

fn stdout_json(result: &ExecResult) -> Result<Value, String> {
    let trimmed = result.stdout.trim();
    if !trimmed.is_empty() {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            if value.get("ok") == Some(&Value::Bool(false)) {
                let message = value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("kbshff reported failure");
                return Err(message.to_string());
            }
            return Ok(value);
        }
    }
    if result.success {
        return Ok(Value::Null);
    }
    Err(failure_text(result))
}

/// `config` JSON envelopes keep the payload under `data`.
#[must_use]
pub fn envelope_data(value: Value) -> Value {
    value.get("data").cloned().unwrap_or(value)
}

fn failure_text(result: &ExecResult) -> String {
    let stderr = result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = result.stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    format!("kbshff exited with status {}", result.code.unwrap_or(-1))
}
