use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub const MASTER_PROMPT_FILE: &str = "master_prompt.md";
pub const SKILLS_FILE: &str = "skills.dsl";
pub const RULES_FILE: &str = "rules.md";

#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub master_prompt: String,
    pub skills_source: String,
    pub rules: String,
}

#[derive(Debug, Error)]
#[allow(clippy::enum_variant_names)] // *File suffixes are intentional for settings I/O errors.
pub enum SettingsError {
    #[error("failed to read settings file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write settings file `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("settings file `{0}` must not be empty")]
    EmptyFile(String),
}

/// Loads required and optional agent settings from a directory.
///
/// # Errors
///
/// Returns [`SettingsError`] if required files are missing, unreadable, or empty.
pub fn load_settings(settings_dir: &Path) -> Result<AgentSettings, SettingsError> {
    let master_path = settings_dir.join(MASTER_PROMPT_FILE);
    let skills_path = settings_dir.join(SKILLS_FILE);
    let rules_path = settings_dir.join(RULES_FILE);

    let master_prompt = read_required(&master_path)?;
    let skills_source = read_required(&skills_path)?;
    let rules = match fs::read_to_string(&rules_path) {
        Ok(value) => value,
        Err(source) if source.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(SettingsError::ReadFile {
                path: rules_path.display().to_string(),
                source,
            });
        }
    };

    Ok(AgentSettings {
        master_prompt,
        skills_source,
        rules,
    })
}

/// Atomically writes `master_prompt.md` under `settings_dir`.
///
/// # Errors
///
/// Returns [`SettingsError::EmptyFile`] if `content` is empty/whitespace-only, or
/// [`SettingsError::WriteFile`] on I/O failure.
pub fn write_master_prompt(settings_dir: &Path, content: &str) -> Result<(), SettingsError> {
    write_required(settings_dir, MASTER_PROMPT_FILE, content)
}

/// Atomically writes `skills.dsl` under `settings_dir`.
///
/// # Errors
///
/// Returns [`SettingsError::EmptyFile`] if `content` is empty/whitespace-only, or
/// [`SettingsError::WriteFile`] on I/O failure.
pub fn write_skills_source(settings_dir: &Path, content: &str) -> Result<(), SettingsError> {
    write_required(settings_dir, SKILLS_FILE, content)
}

/// Atomically writes `rules.md` under `settings_dir`. Empty content is allowed.
///
/// # Errors
///
/// Returns [`SettingsError::WriteFile`] on I/O failure.
pub fn write_rules(settings_dir: &Path, content: &str) -> Result<(), SettingsError> {
    let path = settings_dir.join(RULES_FILE);
    atomic_write(&path, content).map_err(|source| SettingsError::WriteFile {
        path: path.display().to_string(),
        source,
    })
}

/// Deletes `rules.md` under `settings_dir` if it exists.
///
/// # Errors
///
/// Returns [`SettingsError::WriteFile`] on I/O failure other than not found.
pub fn clear_rules(settings_dir: &Path) -> Result<(), SettingsError> {
    let path = settings_dir.join(RULES_FILE);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SettingsError::WriteFile {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn read_required(path: &Path) -> Result<String, SettingsError> {
    let value = fs::read_to_string(path).map_err(|source| SettingsError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    if value.trim().is_empty() {
        return Err(SettingsError::EmptyFile(path.display().to_string()));
    }
    Ok(value)
}

fn write_required(
    settings_dir: &Path,
    file_name: &str,
    content: &str,
) -> Result<(), SettingsError> {
    let path = settings_dir.join(file_name);
    if content.trim().is_empty() {
        return Err(SettingsError::EmptyFile(path.display().to_string()));
    }
    atomic_write(&path, content).map_err(|source| SettingsError::WriteFile {
        path: path.display().to_string(),
        source,
    })
}

fn atomic_write(path: &Path, content: &str) -> io::Result<()> {
    let temp_path = temp_path_for(path);
    fs::write(&temp_path, content)?;
    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            // Windows cannot rename over an existing destination.
            if path.exists() {
                fs::remove_file(path)?;
                match fs::rename(&temp_path, path) {
                    Ok(()) => Ok(()),
                    Err(rename_err) => {
                        let _ = fs::remove_file(&temp_path);
                        Err(rename_err)
                    }
                }
            } else {
                let _ = fs::remove_file(&temp_path);
                Err(err)
            }
        }
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    PathBuf::from(temp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_required_files_and_allows_missing_rules() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join(MASTER_PROMPT_FILE), "master").expect("write master");
        fs::write(
            dir.path().join(SKILLS_FILE),
            r#"skill "x" { policy: "safe" allowed_tools: ["home.read"] }"#,
        )
        .expect("write skills");

        let settings = load_settings(dir.path()).expect("load settings");
        assert_eq!(settings.master_prompt, "master");
        assert!(settings.rules.is_empty());
    }

    #[test]
    fn rejects_empty_required_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join(MASTER_PROMPT_FILE), " ").expect("write master");
        fs::write(dir.path().join(SKILLS_FILE), "skills").expect("write skills");

        assert!(matches!(
            load_settings(dir.path()),
            Err(SettingsError::EmptyFile(_))
        ));
    }

    #[test]
    fn write_helpers_round_trip_and_reject_empty() {
        let dir = tempfile::tempdir().expect("temp dir");

        write_master_prompt(dir.path(), "master").expect("write master");
        write_skills_source(
            dir.path(),
            r#"skill "x" { policy: "safe" allowed_tools: ["home.read"] }"#,
        )
        .expect("write skills");
        write_rules(dir.path(), "be careful").expect("write rules");

        let settings = load_settings(dir.path()).expect("load settings");
        assert_eq!(settings.master_prompt, "master");
        assert!(settings.skills_source.contains("home.read"));
        assert_eq!(settings.rules, "be careful");

        assert!(matches!(
            write_master_prompt(dir.path(), "  "),
            Err(SettingsError::EmptyFile(_))
        ));
        assert!(matches!(
            write_skills_source(dir.path(), ""),
            Err(SettingsError::EmptyFile(_))
        ));

        clear_rules(dir.path()).expect("clear rules");
        assert!(!dir.path().join(RULES_FILE).exists());
        clear_rules(dir.path()).expect("clear missing rules is ok");
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(MASTER_PROMPT_FILE);
        fs::write(&path, "old").expect("seed");
        write_master_prompt(dir.path(), "new").expect("replace");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new");
        assert!(!temp_path_for(&path).exists());
    }
}
