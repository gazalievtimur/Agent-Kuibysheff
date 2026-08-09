use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::access::paths::{is_within_root, relative_components};
use crate::access::{HomeFsPolicy, PathOperation, ProgramAlias};
use crate::agent::RunCancel;
use crate::sandbox::{
    absolute_home_grants, build_sandbox_env, SandboxError, SandboxOutput, SandboxRunner,
    SandboxSpec,
};
use crate::tools::HomeFsError;

const DEFAULT_MAX_CHARS: usize = 50_000;
const MAX_READ_CHARS: usize = 200_000;
const DEFAULT_RUN_TIMEOUT_MS: u64 = 30_000;

pub struct HomeFs {
    root: PathBuf,
    policy: HomeFsPolicy,
    sandbox: Arc<SandboxRunner>,
    run_cancel: RunCancel,
}

impl HomeFs {
    /// Creates a sandboxed filesystem rooted at `root` with the given path/run policy.
    ///
    /// When `policy.programs` is non-empty, `sandbox` must successfully [`SandboxRunner::probe`];
    /// production runners without a native backend fail closed at construction.
    ///
    /// # Errors
    ///
    /// Returns [`HomeFsError`] if the directory cannot be created, is not a directory, or the
    /// sandbox required for configured programs is unavailable.
    pub async fn new(
        root: &Path,
        policy: HomeFsPolicy,
        sandbox: Arc<SandboxRunner>,
        run_cancel: RunCancel,
    ) -> Result<Self, HomeFsError> {
        fs::create_dir_all(root)
            .await
            .map_err(|error| home_io("create_dir_all", root, error))?;
        let root = fs::canonicalize(root)
            .await
            .map_err(|error| home_io("canonicalize", root, error))?;
        if !fs::metadata(&root)
            .await
            .map_err(|error| home_io("metadata", &root, error))?
            .is_dir()
        {
            return Err(HomeFsError::PathDenied {
                path: root.display().to_string(),
                error: "home is not a directory".to_string(),
            });
        }
        if !policy.programs.is_empty() {
            let probe_sandbox = Arc::clone(&sandbox);
            tokio::task::spawn_blocking(move || probe_sandbox.probe())
                .await
                .map_err(|error| HomeFsError::Io {
                    operation: "spawn_blocking".to_string(),
                    path: String::new(),
                    source: std::io::Error::other(error.to_string()),
                })?
                .map_err(map_sandbox_error)?;
        }
        Ok(Self {
            root,
            policy,
            sandbox,
            run_cancel,
        })
    }

    /// Dispatches a home filesystem tool call.
    ///
    /// # Errors
    ///
    /// Returns [`HomeFsError`] for invalid arguments, paths, or I/O failures.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, HomeFsError> {
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
            "run" => {
                let args: RunArgs = decode_args(tool, arguments)?;
                self.run(args.program, args.args, args.timeout_ms).await
            }
            _ => Err(HomeFsError::UnknownTool {
                tool: tool.to_string(),
            }),
        }
    }

    async fn list(&self, relative: &Path) -> Result<Value, HomeFsError> {
        self.policy
            .read
            .allows_relative(relative, PathOperation::Read)
            .map_err(|reason| invalid_path(relative, reason))?;
        let path = self.resolve_existing(relative, PathOperation::Read).await?;
        let metadata = fs::metadata(&path)
            .await
            .map_err(|error| home_io("metadata", &path, error))?;
        if !metadata.is_dir() {
            return Err(HomeFsError::PathDenied {
                path: relative.display().to_string(),
                error: "path is not a directory".to_string(),
            });
        }

        let mut reader = fs::read_dir(&path)
            .await
            .map_err(|error| home_io("read_dir", &path, error))?;
        let mut entries = Vec::with_capacity(32);
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|error| home_io("read_dir", &path, error))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| home_io("file_type", &entry.path(), error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = if file_type.is_dir() {
                "directory"
            } else if file_type.is_file() {
                "file"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "other"
            };
            entries.push((name, kind));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let entries: Vec<Value> = entries
            .into_iter()
            .map(|(name, kind)| json!({ "name": name, "kind": kind }))
            .collect();

        Ok(json!({
            "path": display_relative(relative),
            "entries": entries
        }))
    }

    async fn read(&self, relative: &Path, max_chars: Option<usize>) -> Result<Value, HomeFsError> {
        self.policy
            .read
            .allows_relative(relative, PathOperation::Read)
            .map_err(|reason| invalid_path(relative, reason))?;
        let path = self.resolve_existing(relative, PathOperation::Read).await?;
        let max_chars = max_chars
            .unwrap_or(DEFAULT_MAX_CHARS)
            .clamp(1, MAX_READ_CHARS);
        // Hard ceiling uses MAX_READ_CHARS so a small max_chars only truncates, not rejects.
        let max_bytes = (MAX_READ_CHARS as u64).saturating_mul(4);
        let file_len = fs::metadata(&path)
            .await
            .map_err(|error| home_io("metadata", &path, error))?
            .len();
        if file_len > max_bytes {
            return Err(invalid_path(
                relative,
                format!(
                    "file size {file_len} bytes exceeds read limit of {max_bytes} bytes \
                     ({MAX_READ_CHARS} chars at UTF-8 worst case)"
                ),
            ));
        }
        let content = fs::read_to_string(&path)
            .await
            .map_err(|error| home_io("read_to_string", &path, error))?;
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

    async fn write(&self, relative: &Path, content: &str) -> Result<Value, HomeFsError> {
        self.policy
            .write
            .allows_relative(relative, PathOperation::Write)
            .map_err(|reason| invalid_path(relative, reason))?;
        relative_components(relative).map_err(|reason| invalid_path(relative, reason))?;
        let file_name = relative
            .file_name()
            .ok_or_else(|| invalid_path(relative, "path must name a file"))?;
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = self.ensure_directories(parent).await?;
        let destination = parent.join(file_name);

        if fs::symlink_metadata(&destination).await.is_ok() {
            let canonical = fs::canonicalize(&destination)
                .await
                .map_err(|error| home_io("canonicalize", &destination, error))?;
            self.ensure_within_home(&canonical, relative)?;
            self.ensure_grant_for_canonical(&canonical, PathOperation::Write, relative)?;
            if fs::metadata(&canonical)
                .await
                .map_err(|error| home_io("metadata", &canonical, error))?
                .is_dir()
            {
                return Err(invalid_path(relative, "path is a directory"));
            }
        }

        fs::write(&destination, content)
            .await
            .map_err(|error| home_io("write", &destination, error))?;
        Ok(json!({
            "path": display_relative(relative),
            "bytes_written": content.len()
        }))
    }

    async fn run(
        &self,
        program: String,
        args: Vec<String>,
        timeout_ms: Option<u64>,
    ) -> Result<Value, HomeFsError> {
        if program.trim().is_empty() {
            return Err(HomeFsError::InvalidArguments {
                error: "`program` must not be empty".to_string(),
            });
        }

        let alias = ProgramAlias::parse(&program)
            .map_err(|reason| HomeFsError::InvalidArguments { error: reason })?;
        let program_policy =
            self.policy
                .programs
                .get(&alias)
                .ok_or_else(|| HomeFsError::ProgramDenied {
                    program: alias.to_string(),
                    reason: "not configured in access policy".to_string(),
                })?;

        if args.len() > self.policy.max_args {
            return Err(HomeFsError::InvalidArguments {
                error: format!(
                    "`args` length {} exceeds access.run.max_args {}",
                    args.len(),
                    self.policy.max_args
                ),
            });
        }
        let mut total_arg_chars = 0usize;
        for arg in &args {
            if arg.contains('\0') {
                return Err(HomeFsError::InvalidArguments {
                    error: "`args` must not contain NUL bytes".to_string(),
                });
            }
            total_arg_chars = total_arg_chars.saturating_add(arg.chars().count());
            if total_arg_chars > self.policy.max_arg_chars {
                return Err(HomeFsError::InvalidArguments {
                    error: format!(
                        "`args` exceed access.run.max_arg_chars {}",
                        self.policy.max_arg_chars
                    ),
                });
            }
        }

        let timeout_ms = timeout_ms
            .unwrap_or(DEFAULT_RUN_TIMEOUT_MS)
            .clamp(1, self.policy.max_timeout_ms);
        // Align sandbox kill timer with remaining run wall-clock budget when armed.
        let timeout_ms = match self.run_cancel.remaining() {
            Some(remaining) => {
                let remaining_ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
                timeout_ms.min(remaining_ms.max(1))
            }
            None => timeout_ms,
        };

        let env = build_sandbox_env(&self.root, &program_policy.inherit_env);
        tracing::info!(
            capability = "home.run",
            alias = %alias,
            timeout_ms,
            "home.run dispatching to sandbox"
        );
        let spec = SandboxSpec {
            alias,
            executable: program_policy.executable.as_path().to_path_buf(),
            argv: args,
            cwd: self.root.clone(),
            env,
            home_read: absolute_home_grants(&self.root, &self.policy.read),
            home_write: absolute_home_grants(&self.root, &self.policy.write),
            runtime_read_roots: program_policy.runtime_read_roots.clone(),
            deadline: Duration::from_millis(timeout_ms),
            max_output_chars: self.policy.max_output_chars,
            allow_children: program_policy.allow_children,
        };

        let output = self.sandbox.run(spec).await.map_err(map_sandbox_error)?;
        Ok(sandbox_output_json(&output))
    }

    async fn resolve_existing(
        &self,
        relative: &Path,
        operation: PathOperation,
    ) -> Result<PathBuf, HomeFsError> {
        relative_components(relative).map_err(|reason| invalid_path(relative, reason))?;
        let candidate = self.root.join(relative);
        let canonical = fs::canonicalize(&candidate)
            .await
            .map_err(|error| home_io("canonicalize", &candidate, error))?;
        self.ensure_within_home(&canonical, relative)?;
        self.ensure_grant_for_canonical(&canonical, operation, relative)?;
        Ok(canonical)
    }

    async fn ensure_directories(&self, relative: &Path) -> Result<PathBuf, HomeFsError> {
        relative_components(relative).map_err(|reason| invalid_path(relative, reason))?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                continue;
            };
            let next = current.join(part);
            match fs::symlink_metadata(&next).await {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(invalid_path(
                            relative,
                            "symlink parent components are not allowed",
                        ));
                    }
                    let canonical = fs::canonicalize(&next)
                        .await
                        .map_err(|error| home_io("canonicalize", &next, error))?;
                    self.ensure_within_home(&canonical, relative)?;
                    if !fs::metadata(&canonical)
                        .await
                        .map_err(|error| home_io("metadata", &canonical, error))?
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
                    // Creating under write grant: the final file path was already checked;
                    // intermediate dirs must remain inside home.
                    fs::create_dir(&next)
                        .await
                        .map_err(|error| home_io("create_dir", &next, error))?;
                    current = fs::canonicalize(&next)
                        .await
                        .map_err(|error| home_io("canonicalize", &next, error))?;
                    self.ensure_within_home(&current, relative)?;
                }
                Err(error) => return Err(home_io("symlink_metadata", &next, error)),
            }
        }
        Ok(current)
    }

    fn ensure_within_home(&self, canonical: &Path, requested: &Path) -> Result<(), HomeFsError> {
        if crate::access::is_denied_protected_path(None, canonical) {
            return Err(invalid_path(
                requested,
                crate::access::PROTECTED_DENY_REASON,
            ));
        }
        if is_within_root(&self.root, canonical) {
            Ok(())
        } else {
            Err(invalid_path(requested, "path resolves outside home"))
        }
    }

    fn ensure_grant_for_canonical(
        &self,
        canonical: &Path,
        operation: PathOperation,
        requested: &Path,
    ) -> Result<(), HomeFsError> {
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| invalid_path(requested, "path resolves outside home"))?;
        let scope = match operation {
            PathOperation::Read | PathOperation::Execute => &self.policy.read,
            PathOperation::Write => &self.policy.write,
        };
        scope
            .allows_relative(relative, operation)
            .map_err(|reason| invalid_path(requested, reason))
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

#[derive(Deserialize)]
struct RunArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    timeout_ms: Option<u64>,
}

fn default_list_path() -> String {
    ".".to_string()
}

fn sandbox_output_json(output: &SandboxOutput) -> Value {
    json!({
        "stdout": output.stdout,
        "stderr": output.stderr,
        "stdout_truncated": output.stdout_truncated,
        "stderr_truncated": output.stderr_truncated,
        "exit_code": output.exit_code,
        "timed_out": output.timed_out
    })
}

fn map_sandbox_error(error: SandboxError) -> HomeFsError {
    match error {
        SandboxError::Unavailable { reason } => HomeFsError::SandboxUnavailable { reason },
        SandboxError::PolicyDenied { reason } => HomeFsError::ProgramDenied {
            program: "home.run".to_string(),
            reason,
        },
        other => HomeFsError::Sandbox { source: other },
    }
}

fn decode_args<T: for<'de> Deserialize<'de>>(tool: &str, value: Value) -> Result<T, HomeFsError> {
    serde_json::from_value(value).map_err(|error| HomeFsError::InvalidArguments {
        error: format!("home.{tool}: {error}"),
    })
}

fn invalid_path(path: &Path, error: impl Into<String>) -> HomeFsError {
    HomeFsError::PathDenied {
        path: path.display().to_string(),
        error: error.into(),
    }
}

fn home_io(operation: &str, path: &Path, error: std::io::Error) -> HomeFsError {
    HomeFsError::Io {
        operation: operation.to_string(),
        path: path.display().to_string(),
        source: error,
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
    use crate::access::paths::PathGrantScope;
    use crate::access::{CanonicalRoot, ProgramAlias, RelativeGrant, ResolvedProgramPolicy};
    use crate::agent::RunCancel;
    use crate::sandbox::{MockBackend, SandboxOutput, UnavailableBackend};
    use std::collections::BTreeMap;

    fn mock_sandbox() -> Arc<SandboxRunner> {
        Arc::new(SandboxRunner::with_backend(Arc::new(
            MockBackend::with_output(SandboxOutput {
                stdout: "hello-run\n".to_string(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                exit_code: Some(0),
                timed_out: false,
            }),
        )))
    }

    fn unavailable_sandbox() -> Arc<SandboxRunner> {
        Arc::new(SandboxRunner::with_backend(Arc::new(UnavailableBackend {
            reason: "test sandbox unavailable".to_string(),
        })))
    }

    fn legacy_home() -> HomeFsPolicy {
        HomeFsPolicy::legacy()
    }

    async fn python_program_policy(dir: &Path) -> HomeFsPolicy {
        let mut policy = HomeFsPolicy::legacy();
        let exe = dir.join("python-stub");
        fs::write(&exe, "stub").await.expect("exe");
        let executable = CanonicalRoot::canonicalize(&exe).expect("canonicalize exe");
        let mut programs = BTreeMap::new();
        programs.insert(
            ProgramAlias::parse("python").unwrap(),
            ResolvedProgramPolicy {
                alias: ProgramAlias::parse("python").unwrap(),
                executable,
                runtime_read_roots: Vec::new(),
                inherit_env: Vec::new(),
                allow_children: false,
            },
        );
        policy.programs = programs;
        policy
    }

    fn strict_home(read: &[&str], write: &[&str]) -> HomeFsPolicy {
        let mut policy = HomeFsPolicy::legacy();
        policy.read = PathGrantScope::strict(
            read.iter()
                .map(|g| RelativeGrant::parse(g).unwrap())
                .collect(),
        );
        policy.write = PathGrantScope::strict(
            write
                .iter()
                .map(|g| RelativeGrant::parse(g).unwrap())
                .collect(),
        );
        policy
    }

    #[tokio::test]
    async fn writes_reads_and_lists_inside_home() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = HomeFs::new(
            dir.path(),
            legacy_home(),
            unavailable_sandbox(),
            RunCancel::new(),
        )
        .await
        .expect("home");

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
        let home = HomeFs::new(
            dir.path(),
            legacy_home(),
            unavailable_sandbox(),
            RunCancel::new(),
        )
        .await
        .expect("home");

        let error = home
            .call("write", json!({"path": "../outside.txt", "content": "no"}))
            .await
            .expect_err("must reject traversal");
        assert!(matches!(error, HomeFsError::PathDenied { .. }));
    }

    #[tokio::test]
    async fn rejects_sibling_prefix_outside_grant() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("outside"))
            .await
            .expect("outside");
        fs::write(dir.path().join("outside/secret.txt"), "no")
            .await
            .expect("secret");
        let home = HomeFs::new(
            dir.path(),
            strict_home(&["out"], &["out"]),
            unavailable_sandbox(),
            RunCancel::new(),
        )
        .await
        .expect("home");

        let error = home
            .call("read", json!({"path": "outside/secret.txt"}))
            .await
            .expect_err("sibling prefix");
        assert!(matches!(error, HomeFsError::PathDenied { .. }));
    }

    #[tokio::test]
    async fn allows_write_under_missing_target_when_grant_matches() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = HomeFs::new(
            dir.path(),
            strict_home(&["out"], &["out"]),
            unavailable_sandbox(),
            RunCancel::new(),
        )
        .await
        .expect("home");

        home.call(
            "write",
            json!({"path": "out/new/file.txt", "content": "ok"}),
        )
        .await
        .expect("write missing target");
        assert_eq!(
            fs::read_to_string(dir.path().join("out/new/file.txt"))
                .await
                .expect("read"),
            "ok"
        );
    }

    #[tokio::test]
    async fn runs_command_via_sandbox_runner() {
        let dir = tempfile::tempdir().expect("temp dir");
        let policy = python_program_policy(dir.path()).await;

        let home = HomeFs::new(dir.path(), policy, mock_sandbox(), RunCancel::new())
            .await
            .expect("home");
        let result = home
            .call(
                "run",
                json!({
                    "program": "python",
                    "args": ["-c", "print(1)"],
                    "timeout_ms": 10_000
                }),
            )
            .await
            .expect("run");

        assert_eq!(result["timed_out"], false);
        assert_eq!(result["exit_code"], 0);
        assert!(
            result["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("hello-run"),
            "stdout was: {}",
            result["stdout"]
        );
    }

    #[tokio::test]
    async fn refuses_programs_without_working_sandbox() {
        let dir = tempfile::tempdir().expect("temp dir");
        let policy = python_program_policy(dir.path()).await;

        let Err(err) =
            HomeFs::new(dir.path(), policy, unavailable_sandbox(), RunCancel::new()).await
        else {
            panic!("must require working sandbox");
        };
        assert!(matches!(err, HomeFsError::SandboxUnavailable { .. }));
    }

    #[tokio::test]
    async fn rejects_empty_program() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = HomeFs::new(
            dir.path(),
            legacy_home(),
            unavailable_sandbox(),
            RunCancel::new(),
        )
        .await
        .expect("home");

        let error = home
            .call("run", json!({"program": "  ", "args": []}))
            .await
            .expect_err("must reject empty program");
        assert!(matches!(error, HomeFsError::InvalidArguments { .. }));
    }

    #[tokio::test]
    async fn rejects_unknown_program_alias() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = HomeFs::new(
            dir.path(),
            legacy_home(),
            unavailable_sandbox(),
            RunCancel::new(),
        )
        .await
        .expect("home");
        let error = home
            .call("run", json!({"program": "python", "args": []}))
            .await
            .expect_err("unknown alias");
        assert!(matches!(error, HomeFsError::ProgramDenied { .. }));
    }
}
