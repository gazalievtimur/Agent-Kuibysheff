use std::fs;
use std::path::Path;

use thiserror::Error;

const MASTER_PROMPT_FILE: &str = "master_prompt.md";
const SKILLS_FILE: &str = "skills.dsl";
const RULES_FILE: &str = "rules.md";

#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub master_prompt: String,
    pub skills_source: String,
    pub rules: String,
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("failed to read settings file `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("settings file `{0}` must not be empty")]
    EmptyFile(String),
}

pub fn load_settings(settings_dir: &Path) -> Result<AgentSettings, SettingsError> {
    let master_path = settings_dir.join(MASTER_PROMPT_FILE);
    let skills_path = settings_dir.join(SKILLS_FILE);
    let rules_path = settings_dir.join(RULES_FILE);

    let master_prompt = read_required(&master_path)?;
    let skills_source = read_required(&skills_path)?;
    let rules = match fs::read_to_string(&rules_path) {
        Ok(value) => value,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
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
}
