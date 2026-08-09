//! `config import` — copy external config/settings into the protected profile.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::access::ensure_protected_profile_dirs;
use crate::cli::{ConfigArgs, ImportArgs};
use crate::config::{
    bootstrap_app_config, ensure_access_present, load_config, parse_config_payload, save_config,
    ConfigSafetyValidator,
};
use crate::project_paths::{kuibysheff_root, resolve_agent_identity, AGENT_CONFIG_FILE};
use crate::settings::{load_settings, MASTER_PROMPT_FILE, RULES_FILE, SKILLS_FILE};
use crate::skills::dsl::SkillsCatalog;

use super::common::{copy_contents, dir_non_empty, reject_symlink, CommonError};
use super::{emit_ok, ConfigCmdError};

const AGENT_CONFIG_EXAMPLE_FILE: &str = "agent-config.example.yaml";

#[derive(Debug, Serialize)]
struct ImportResult {
    agent: String,
    profile_dir: String,
    imported: Vec<String>,
    source: String,
    /// True when a minimal local config was created because the profile had none.
    bootstrapped: bool,
    /// True when the external payload declared `access` and it was written through.
    access_imported: bool,
}

/// Import a config file or settings directory into the protected agent profile.
///
/// External paths are treated as untrusted payloads until contents are installed into
/// the protected profile and validated there. Missing local config is bootstrapped with
/// [`crate::access::AccessPolicyConfig::minimal_profile`] first. External `access` is
/// imported when present; when absent, minimal access is filled before validation.
///
/// # Errors
///
/// Returns validation, safety, force, symlink, or I/O failures.
pub fn run(args: &ConfigArgs, import_args: &ImportArgs) -> Result<(), ConfigCmdError> {
    let paths = resolve_agent_identity(&args.identity.project_root, &args.identity.agent, None)?;
    let source = &import_args.from;

    reject_symlink(source).map_err(ConfigCmdError::from)?;
    let meta = fs::symlink_metadata(source).map_err(|source_err| {
        ConfigCmdError::from(CommonError::ReadFile {
            path: source.display().to_string(),
            source: source_err,
        })
    })?;

    ensure_protected_profile_dirs(&paths.profile_dir).map_err(|source_err| {
        ConfigCmdError::from(CommonError::WriteFile {
            path: paths.profile_dir.display().to_string(),
            source: source_err,
        })
    })?;

    let mut bootstrapped = false;
    if !paths.config.is_file() {
        save_config(&paths.config, &bootstrap_app_config()).map_err(ConfigCmdError::from)?;
        bootstrapped = true;
    }

    let staging_root = kuibysheff_root(&args.identity.project_root)
        .join(".import-staging")
        .join(&args.identity.agent);
    if staging_root.exists() {
        fs::remove_dir_all(&staging_root).map_err(|source_err| {
            ConfigCmdError::from(CommonError::WriteFile {
                path: staging_root.display().to_string(),
                source: source_err,
            })
        })?;
    }
    fs::create_dir_all(&staging_root).map_err(|source_err| {
        ConfigCmdError::from(CommonError::WriteFile {
            path: staging_root.display().to_string(),
            source: source_err,
        })
    })?;
    let stage_root = staging_root.as_path();

    let mut imported = Vec::new();
    let mut access_imported = false;

    let stage_result = if meta.is_file() {
        stage_config_file(source, stage_root, &mut imported, &mut access_imported)
    } else if meta.is_dir() {
        stage_directory(source, stage_root, &mut imported, &mut access_imported)
    } else {
        Err(ConfigCmdError::message(format!(
            "`{}` is neither a file nor a directory",
            source.display()
        )))
    };

    if let Err(err) = stage_result {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(err);
    }

    if let Err(err) = validate_staging(stage_root, &paths.config) {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(err);
    }

    // Profile may already contain bootstrap config; treat that as non-empty.
    if dir_non_empty(&paths.profile_dir)? && !import_args.force {
        // Allow first-time import into a freshly bootstrapped profile (config only).
        let only_bootstrap = bootstrapped && profile_only_has_bootstrap(&paths.profile_dir)?;
        if !only_bootstrap {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(ConfigCmdError::message(format!(
                "profile `{}` is not empty; pass `--force` to overwrite",
                paths.profile_dir.display()
            )));
        }
    }

    if let Err(err) = install_staging(stage_root, &paths.profile_dir, &imported) {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(err);
    }
    let _ = fs::remove_dir_all(&staging_root);

    // Final authority check on the written protected profile.
    validate_installed_profile(&paths.profile_dir, &imported)?;

    let result = ImportResult {
        agent: paths.agent_id.clone(),
        profile_dir: paths.profile_dir.display().to_string(),
        imported,
        source: source.display().to_string(),
        bootstrapped,
        access_imported,
    };
    emit_ok(args.format, "import", "done", &result)
}

fn profile_only_has_bootstrap(profile_dir: &Path) -> Result<bool, ConfigCmdError> {
    let entries = fs::read_dir(profile_dir).map_err(|source| {
        ConfigCmdError::from(CommonError::ReadFile {
            path: profile_dir.display().to_string(),
            source,
        })
    })?;
    let mut saw_config = false;
    for entry in entries {
        let entry = entry.map_err(|source| {
            ConfigCmdError::from(CommonError::ReadFile {
                path: profile_dir.display().to_string(),
                source,
            })
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == AGENT_CONFIG_FILE {
            saw_config = true;
            continue;
        }
        // Ignore empty placeholder dirs created by ensure_protected_profile_dirs.
        let meta = entry.metadata().map_err(|source| {
            ConfigCmdError::from(CommonError::ReadFile {
                path: entry.path().display().to_string(),
                source,
            })
        })?;
        if meta.is_dir() {
            let mut child = fs::read_dir(entry.path()).map_err(|source| {
                ConfigCmdError::from(CommonError::ReadFile {
                    path: entry.path().display().to_string(),
                    source,
                })
            })?;
            if child.next().is_none() {
                continue;
            }
        }
        return Ok(false);
    }
    Ok(saw_config)
}

fn stage_config_file(
    source: &Path,
    stage_root: &Path,
    imported: &mut Vec<String>,
    access_imported: &mut bool,
) -> Result<(), ConfigCmdError> {
    let dest = stage_root.join(AGENT_CONFIG_FILE);
    write_staged_config_from_payload(source, &dest, access_imported)?;
    imported.push(AGENT_CONFIG_FILE.to_string());
    Ok(())
}

fn stage_directory(
    source: &Path,
    stage_root: &Path,
    imported: &mut Vec<String>,
    access_imported: &mut bool,
) -> Result<(), ConfigCmdError> {
    let master = source.join(MASTER_PROMPT_FILE);
    let skills = source.join(SKILLS_FILE);
    if !master.is_file() {
        return Err(ConfigCmdError::message(format!(
            "directory import requires `{}`",
            MASTER_PROMPT_FILE
        )));
    }
    if !skills.is_file() {
        return Err(ConfigCmdError::message(format!(
            "directory import requires `{}`",
            SKILLS_FILE
        )));
    }

    copy_contents(&master, &stage_root.join(MASTER_PROMPT_FILE))?;
    imported.push(MASTER_PROMPT_FILE.to_string());
    copy_contents(&skills, &stage_root.join(SKILLS_FILE))?;
    imported.push(SKILLS_FILE.to_string());

    let rules = source.join(RULES_FILE);
    if rules.is_file() {
        copy_contents(&rules, &stage_root.join(RULES_FILE))?;
        imported.push(RULES_FILE.to_string());
    }

    if let Some(config_src) = resolve_config_source(source)? {
        let dest = stage_root.join(AGENT_CONFIG_FILE);
        write_staged_config_from_payload(&config_src, &dest, access_imported)?;
        imported.push(AGENT_CONFIG_FILE.to_string());
    }
    Ok(())
}

fn write_staged_config_from_payload(
    source: &Path,
    dest: &Path,
    access_imported: &mut bool,
) -> Result<(), ConfigCmdError> {
    let raw = fs::read_to_string(source).map_err(|source_err| {
        ConfigCmdError::from(CommonError::ReadFile {
            path: source.display().to_string(),
            source: source_err,
        })
    })?;
    // Parse as payload only — do not treat `source` as a live runtime config.
    let mut cfg = parse_config_payload(&raw, source).map_err(ConfigCmdError::from)?;
    *access_imported = ensure_access_present(&mut cfg);
    save_config(dest, &cfg).map_err(ConfigCmdError::from)?;
    Ok(())
}

fn resolve_config_source(dir: &Path) -> Result<Option<PathBuf>, ConfigCmdError> {
    let primary = dir.join(AGENT_CONFIG_FILE);
    if primary.is_file() {
        return Ok(Some(primary));
    }
    let example = dir.join(AGENT_CONFIG_EXAMPLE_FILE);
    if example.is_file() {
        return Ok(Some(example));
    }
    Ok(None)
}

fn validate_staging(stage_root: &Path, profile_config: &Path) -> Result<(), ConfigCmdError> {
    let staged_config = stage_root.join(AGENT_CONFIG_FILE);
    if staged_config.is_file() {
        let (cfg, _) = load_config(&staged_config)?;
        ConfigSafetyValidator::check(&cfg)?;
    } else {
        // Settings-only import: rely on already-written profile/bootstrap config.
        let (cfg, _) = load_config(profile_config)?;
        ConfigSafetyValidator::check(&cfg)?;
    }

    let master = stage_root.join(MASTER_PROMPT_FILE);
    let skills = stage_root.join(SKILLS_FILE);
    if master.is_file() || skills.is_file() {
        if !master.is_file() || !skills.is_file() {
            return Err(ConfigCmdError::message(
                "staged settings are incomplete (need both master_prompt.md and skills.dsl)"
                    .to_string(),
            ));
        }
        let settings = load_settings(stage_root)?;
        SkillsCatalog::parse(&settings.skills_source)?;
    }
    Ok(())
}

fn validate_installed_profile(
    profile_dir: &Path,
    imported: &[String],
) -> Result<(), ConfigCmdError> {
    let config_path = profile_dir.join(AGENT_CONFIG_FILE);
    let (cfg, _) = load_config(&config_path)?;
    ConfigSafetyValidator::check(&cfg)?;

    if imported
        .iter()
        .any(|name| name == MASTER_PROMPT_FILE || name == SKILLS_FILE)
    {
        let settings = load_settings(profile_dir)?;
        SkillsCatalog::parse(&settings.skills_source)?;
    }
    Ok(())
}

fn install_staging(
    stage_root: &Path,
    profile_dir: &Path,
    imported: &[String],
) -> Result<(), ConfigCmdError> {
    for name in imported {
        let src = stage_root.join(name);
        let dest = profile_dir.join(name);
        copy_contents(&src, &dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::AccessPolicyConfig;
    use crate::cli::{AgentIdentityArgs, ConfigArgs, ConfigCommand, ConfigFormat, InitArgs};
    use crate::commands::init;
    use crate::config::load_config;
    use crate::project_paths::AGENT_CONFIG_FILE;
    use crate::settings::{load_settings, MASTER_PROMPT_FILE, SKILLS_FILE};

    fn config_args(root: &Path, agent_id: &str, from: PathBuf, force: bool) -> ConfigArgs {
        ConfigArgs {
            identity: AgentIdentityArgs {
                project_root: root.to_path_buf(),
                agent: agent_id.to_string(),
            },
            format: ConfigFormat::Json,
            command: ConfigCommand::Import(ImportArgs { from, force }),
        }
    }

    fn run_import(args: &ConfigArgs) -> Result<(), ConfigCmdError> {
        match &args.command {
            ConfigCommand::Import(a) => run(args, a),
            _ => unreachable!(),
        }
    }

    fn write_settings_bundle(dir: &Path, with_config: Option<&str>) {
        fs::write(dir.join(MASTER_PROMPT_FILE), "imported master prompt").expect("master");
        fs::write(
            dir.join(SKILLS_FILE),
            r#"skill "workspace" {
  policy: "imported"
  allowed_tools: ["home.list", "home.read", "home.write"]
}"#,
        )
        .expect("skills");
        if let Some(cfg) = with_config {
            fs::write(dir.join(AGENT_CONFIG_FILE), cfg).expect("cfg");
        }
    }

    #[test]
    fn import_directory_roundtrip_after_init() {
        let root = tempfile::tempdir().expect("temp root");
        let agent_id = "import-demo";
        let init_args = InitArgs {
            agent_id: agent_id.to_string(),
            project_root: root.path().to_path_buf(),
            force: false,
            interactive: false,
        };
        init::run(&init_args).expect("init");

        let bundle = tempfile::tempdir().expect("bundle");
        let profile = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let cfg_body = fs::read_to_string(profile.config).expect("read cfg");
        write_settings_bundle(bundle.path(), Some(&cfg_body));

        let args = config_args(root.path(), agent_id, bundle.path().to_path_buf(), true);
        run_import(&args).expect("import");

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let settings = load_settings(&paths.settings_dir).expect("settings");
        assert_eq!(settings.master_prompt, "imported master prompt");
        assert!(settings.skills_source.contains("imported"));
        let _ = load_config(&paths.config).expect("config loads");
    }

    #[test]
    fn import_file_rejects_without_force_on_nonempty() {
        let root = tempfile::tempdir().expect("temp root");
        let agent_id = "import-noforce";
        init::run(&InitArgs {
            agent_id: agent_id.to_string(),
            project_root: root.path().to_path_buf(),
            force: false,
            interactive: false,
        })
        .expect("init");

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let cfg_body = fs::read_to_string(&paths.config).expect("read");
        let external = root.path().join("external-config.yaml");
        fs::write(&external, &cfg_body).expect("external");

        let args = config_args(root.path(), agent_id, external, false);
        let err = run_import(&args).expect_err("should require force");
        assert!(
            err.to_string().contains("--force") || err.to_string().contains("not empty"),
            "{err}"
        );
    }

    #[test]
    fn import_settings_only_bootstraps_missing_config() {
        let root = tempfile::tempdir().expect("temp root");
        let agent_id = "import-bootstrap";
        let bundle = tempfile::tempdir().expect("bundle");
        write_settings_bundle(bundle.path(), None);

        let args = config_args(root.path(), agent_id, bundle.path().to_path_buf(), false);
        run_import(&args).expect("import");

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let settings = load_settings(&paths.settings_dir).expect("settings");
        assert_eq!(settings.master_prompt, "imported master prompt");
        let (cfg, _) = load_config(&paths.config).expect("config loads");
        assert_eq!(cfg.access, Some(AccessPolicyConfig::minimal_profile()));
    }

    #[test]
    fn import_external_access_is_written_after_validation() {
        let root = tempfile::tempdir().expect("temp root");
        let agent_id = "import-access";
        let bundle = tempfile::tempdir().expect("bundle");
        let wide = r#"
provider:
  base_url: "https://example.com/v1"
  model: "m"
  api_key_env: "OPENAI_API_KEY"
  timeout_ms: 1000
limits:
  max_iterations: 3
  max_tokens: 100
  max_duration_sec: 30
access:
  mode: legacy
"#;
        write_settings_bundle(bundle.path(), Some(wide));

        let args = config_args(root.path(), agent_id, bundle.path().to_path_buf(), false);
        run_import(&args).expect("import");

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let (cfg, resolved) = load_config(&paths.config).expect("config loads");
        assert_eq!(
            cfg.access.as_ref().map(|a| a.mode),
            Some(crate::access::AccessModeField::Legacy)
        );
        assert!(resolved.is_legacy());
    }

    #[test]
    fn import_config_without_access_fills_minimal() {
        let root = tempfile::tempdir().expect("temp root");
        let agent_id = "import-no-access";
        let external = root.path().join("no-access.yaml");
        fs::write(
            &external,
            r#"
provider:
  base_url: "https://example.com/v1"
  model: "m"
  api_key_env: "OPENAI_API_KEY"
  timeout_ms: 1000
limits:
  max_iterations: 3
  max_tokens: 100
  max_duration_sec: 30
"#,
        )
        .expect("write");

        let args = config_args(root.path(), agent_id, external, false);
        run_import(&args).expect("import");

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        let (cfg, _) = load_config(&paths.config).expect("config loads");
        assert_eq!(cfg.access, Some(AccessPolicyConfig::minimal_profile()));
    }

    #[test]
    fn import_invalid_staged_config_does_not_install_settings() {
        let root = tempfile::tempdir().expect("temp root");
        let agent_id = "import-bad";
        let bundle = tempfile::tempdir().expect("bundle");
        write_settings_bundle(
            bundle.path(),
            Some(
                r#"
provider:
  base_url: ""
  model: "m"
  api_key_env: "OPENAI_API_KEY"
  timeout_ms: 1000
limits:
  max_iterations: 3
  max_tokens: 100
  max_duration_sec: 30
access:
  mode: legacy
"#,
            ),
        );

        let args = config_args(root.path(), agent_id, bundle.path().to_path_buf(), false);
        let err = run_import(&args).expect_err("invalid config");
        assert!(
            err.to_string().contains("base_url")
                || err.to_string().contains("Validation")
                || err.to_string().contains("provider"),
            "{err}"
        );

        let paths = resolve_agent_identity(root.path(), agent_id, None).expect("paths");
        assert!(
            !paths.settings_dir.join(MASTER_PROMPT_FILE).is_file(),
            "settings must not install when config validation fails"
        );
        // Bootstrap may exist; it must still load.
        let _ = load_config(&paths.config).expect("bootstrap remains valid");
    }
}
