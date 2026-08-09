//! Shared load/save helpers for `config` commands.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::access::ResolvedAccessPolicy;
use crate::cli::AgentIdentityArgs;
use crate::config::{load_config, save_config, AppConfig, ConfigError};
use crate::project_paths::{resolve_agent_identity, AgentPathError, ResolvedAgentPaths};
use crate::settings::{load_settings, AgentSettings, SettingsError};
use crate::skills::dsl::{SkillsCatalog, SkillsError};

/// Loaded agent profile (config + settings + resolved paths).
#[derive(Debug, Clone)]
pub struct LoadedProfile {
    pub paths: ResolvedAgentPaths,
    pub config: AppConfig,
    pub access: ResolvedAccessPolicy,
    pub settings: AgentSettings,
}

/// Errors shared by config management helpers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommonError {
    #[error(transparent)]
    AgentPath(#[from] AgentPathError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Skills(#[from] SkillsError),
    #[error("invalid key=value `{0}` (expected KEY=VALUE)")]
    InvalidKeyValue(String),
    #[error("failed to read `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to write `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("path `{0}` is a symlink; copy contents only (symlinks are rejected)")]
    SymlinkRejected(String),
}

/// Resolve identity and load config + settings from the protected profile.
///
/// # Errors
///
/// Returns path, config, or settings load failures.
pub fn load_profile(identity: &AgentIdentityArgs) -> Result<LoadedProfile, CommonError> {
    let paths = resolve_agent_identity(&identity.project_root, &identity.agent, None)?;
    let (config, access) = load_config(&paths.config)?;
    let settings = load_settings(&paths.settings_dir)?;
    Ok(LoadedProfile {
        paths,
        config,
        access,
        settings,
    })
}

/// Persist `AppConfig` into the profile via [`save_config`] (validate + safety).
///
/// # Errors
///
/// Propagates [`ConfigError`] from save.
pub fn save_profile_config(paths: &ResolvedAgentPaths, cfg: &AppConfig) -> Result<(), CommonError> {
    save_config(&paths.config, cfg)?;
    Ok(())
}

/// Parse skills DSL from loaded settings.
///
/// # Errors
///
/// Returns [`SkillsError`] when the DSL is invalid.
pub fn parse_skills(settings: &AgentSettings) -> Result<SkillsCatalog, CommonError> {
    Ok(SkillsCatalog::parse(&settings.skills_source)?)
}

/// Parse `KEY=VALUE` pairs into a map (later entries win).
///
/// # Errors
///
/// Returns [`CommonError::InvalidKeyValue`] when an entry lacks `=`.
pub fn parse_kv_pairs(pairs: &[String]) -> Result<HashMap<String, String>, CommonError> {
    let mut map = HashMap::with_capacity(pairs.len());
    for raw in pairs {
        let Some((key, value)) = raw.split_once('=') else {
            return Err(CommonError::InvalidKeyValue(raw.clone()));
        };
        if key.is_empty() {
            return Err(CommonError::InvalidKeyValue(raw.clone()));
        }
        map.insert(key.to_string(), value.to_string());
    }
    Ok(map)
}

/// Read a file after rejecting symlinks.
///
/// # Errors
///
/// Symlink or I/O failures.
pub fn read_file_no_symlink(path: &Path) -> Result<Vec<u8>, CommonError> {
    reject_symlink(path)?;
    fs::read(path).map_err(|source| CommonError::ReadFile {
        path: path.display().to_string(),
        source,
    })
}

/// Read UTF-8 text after rejecting symlinks.
///
/// # Errors
///
/// Symlink, I/O, or UTF-8 failures (I/O error with InvalidData).
pub fn read_text_no_symlink(path: &Path) -> Result<String, CommonError> {
    let bytes = read_file_no_symlink(path)?;
    String::from_utf8(bytes).map_err(|err| CommonError::ReadFile {
        path: path.display().to_string(),
        source: io::Error::new(io::ErrorKind::InvalidData, err),
    })
}

/// Reject paths that are symlinks (does not follow the link).
///
/// # Errors
///
/// Returns [`CommonError::SymlinkRejected`] or metadata I/O errors.
pub fn reject_symlink(path: &Path) -> Result<(), CommonError> {
    let meta = fs::symlink_metadata(path).map_err(|source| CommonError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    if meta.file_type().is_symlink() {
        return Err(CommonError::SymlinkRejected(path.display().to_string()));
    }
    Ok(())
}

/// True when `dir` exists and contains at least one entry.
///
/// # Errors
///
/// I/O errors other than not-found.
pub fn dir_non_empty(dir: &Path) -> Result<bool, CommonError> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(CommonError::ReadFile {
            path: dir.display().to_string(),
            source,
        }),
    }
}

/// Copy file contents (not the link) to `dest`, creating parent dirs as needed.
///
/// # Errors
///
/// Symlink rejection or I/O failures.
pub fn copy_contents(src: &Path, dest: &Path) -> Result<(), CommonError> {
    let bytes = read_file_no_symlink(src)?;
    atomic_write_bytes(dest, &bytes)
}

/// Atomically write bytes to `path`.
///
/// # Errors
///
/// I/O failures.
pub fn atomic_write_bytes(path: &Path, contents: &[u8]) -> Result<(), CommonError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| CommonError::WriteFile {
                path: path.display().to_string(),
                source,
            })?;
        }
    }
    let tmp = temp_sibling(path);
    if let Err(source) = fs::write(&tmp, contents) {
        let _ = fs::remove_file(&tmp);
        return Err(CommonError::WriteFile {
            path: path.display().to_string(),
            source,
        });
    }
    if path.exists() {
        if let Err(source) = fs::remove_file(path) {
            let _ = fs::remove_file(&tmp);
            return Err(CommonError::WriteFile {
                path: path.display().to_string(),
                source,
            });
        }
    }
    if let Err(source) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(CommonError::WriteFile {
            path: path.display().to_string(),
            source,
        });
    }
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}
