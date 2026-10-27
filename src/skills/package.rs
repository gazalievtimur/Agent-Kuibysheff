//! Skill packages on disk: `skills/<name>/skill.md` plus companion files.
//!
//! The directory name is the skill name from `skills.dsl`. Companion scripts stay
//! inside that directory. Callers never pass a raw filesystem path into `protected/`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::settings::SettingsError;
use crate::skills::dsl::{SkillsCatalog, SkillsError};

pub const SKILL_PACKAGES_DIR: &str = "skills";
pub const SKILL_BODY_FILE: &str = "skill.md";

/// One file inside a skill package, addressed by a relative `/`-separated path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFileEntry {
    pub relative: String,
    pub canonical: PathBuf,
}

/// Indexed skill folder. `body` is `skill.md`. Companion bytes are not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPackage {
    pub name: String,
    pub root: PathBuf,
    pub body: String,
    pub files: Vec<SkillFileEntry>,
}

/// Loads every skill folder under `settings_dir/skills`.
///
/// A missing `skills/` directory is an empty catalog. A directory without a
/// non-empty `skill.md`, a symlink, or a path that escapes the skill folder fails.
///
/// # Errors
///
/// Returns [`SettingsError::InvalidSkillPackage`] for a malformed tree, or
/// [`SettingsError::ReadFile`] on I/O failure.
pub fn load_skill_packages(
    settings_dir: &Path,
) -> Result<BTreeMap<String, SkillPackage>, SettingsError> {
    let root = settings_dir.join(SKILL_PACKAGES_DIR);
    match fs::symlink_metadata(&root) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => {
            return Err(SettingsError::ReadFile {
                path: root.display().to_string(),
                source,
            });
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(invalid(format!(
                "`{}` must not be a symlink",
                root.display()
            )));
        }
        Ok(meta) if !meta.is_dir() => {
            return Err(invalid(format!(
                "`{}` must be a directory of skill folders",
                root.display()
            )));
        }
        Ok(_) => {}
    }

    let mut packages = BTreeMap::new();
    let entries = fs::read_dir(&root).map_err(|source| SettingsError::ReadFile {
        path: root.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| SettingsError::ReadFile {
            path: root.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|source| SettingsError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Err(invalid(format!(
                "skill entry `{}` must not be a symlink",
                path.display()
            )));
        }
        if !meta.is_dir() {
            return Err(invalid(format!(
                "`{}` must be a skill directory (skill.md plus companion files)",
                path.display()
            )));
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(invalid(format!(
                "skill directory `{}` is not valid UTF-8",
                path.display()
            )));
        };
        validate_skill_dir_name(name).map_err(invalid)?;
        let package = load_one(&path, name)?;
        packages.insert(package.name.clone(), package);
    }
    Ok(packages)
}

/// Rejects skill folders whose names are not declared in `skills.dsl`.
///
/// # Errors
///
/// Returns [`SkillsError::UndeclaredPackage`] when a folder has no matching skill.
pub fn validate_packages_match_catalog(
    packages: &BTreeMap<String, SkillPackage>,
    catalog: &SkillsCatalog,
) -> Result<(), SkillsError> {
    for name in packages.keys() {
        if !catalog.skills.iter().any(|skill| skill.name == *name) {
            return Err(SkillsError::UndeclaredPackage(name.clone()));
        }
    }
    Ok(())
}

/// Parses `skills.dsl` and checks that every skill folder is declared there.
///
/// # Errors
///
/// Returns [`SkillsError`] on a DSL or package-name mismatch.
pub fn catalog_from_settings(
    skills_source: &str,
    packages: &BTreeMap<String, SkillPackage>,
) -> Result<SkillsCatalog, SkillsError> {
    let catalog = SkillsCatalog::parse(skills_source)?;
    validate_packages_match_catalog(packages, &catalog)?;
    Ok(catalog)
}

/// Skill folder names are a single path segment: letters, digits, spaces, `_`, `-`.
///
/// # Errors
///
/// Returns a reason when `name` could escape the skill packages directory.
pub fn validate_skill_dir_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name != name.trim() {
        return Err(
            "skill directory name must not be empty or have surrounding whitespace".to_string(),
        );
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("skill directory name must not be empty".to_string());
    };
    if !first.is_alphanumeric() {
        return Err(
            "skill directory name must start with a letter or digit and contain only letters, digits, spaces, '_' or '-'"
                .to_string(),
        );
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == ' ') {
        return Err(
            "skill directory name must start with a letter or digit and contain only letters, digits, spaces, '_' or '-'"
                .to_string(),
        );
    }
    if is_windows_reserved(name) {
        return Err(format!("skill directory name `{name}` is reserved"));
    }
    Ok(())
}

/// Relative companion path using `/`. Rejects empty, absolute, and `..` segments.
///
/// # Errors
///
/// Returns a reason when `path` is not a safe relative file path.
pub fn validate_relative_skill_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\0') {
        return Err("skill file path must be relative".to_string());
    }
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    if parts.is_empty() {
        return Err("skill file path must name a file".to_string());
    }
    for part in &parts {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err("skill file path must stay inside the skill directory".to_string());
        }
        if part.ends_with(' ') || part.ends_with('.') {
            return Err(format!("skill file path component `{part}` is not allowed"));
        }
        if part
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        {
            return Err(format!("skill file path component `{part}` is not allowed"));
        }
        if is_windows_reserved(part) {
            return Err(format!("skill file path component `{part}` is reserved"));
        }
    }
    let as_path = Path::new(path);
    if as_path.is_absolute()
        || as_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("skill file path must stay inside the skill directory".to_string());
    }
    Ok(())
}

/// Writes `skills/<name>/skill.md`, creating the skill directory when needed.
///
/// # Errors
///
/// Rejects an unsafe name, empty content, a symlink, or I/O failure.
pub fn write_body(settings_dir: &Path, name: &str, content: &str) -> Result<(), SettingsError> {
    validate_skill_dir_name(name).map_err(invalid)?;
    if content.trim().is_empty() {
        return Err(invalid(format!(
            "skill `{name}` skill.md must not be empty"
        )));
    }
    let dir = skill_dir(settings_dir, name);
    fs::create_dir_all(&dir).map_err(|source| SettingsError::WriteFile {
        path: dir.display().to_string(),
        source,
    })?;
    let path = dir.join(SKILL_BODY_FILE);
    reject_existing_symlink(&path)?;
    atomic_write(&path, content)
}

/// Deletes `skill.md`. Fails while companion files remain so the folder stays loadable.
///
/// An empty skill directory is removed.
///
/// # Errors
///
/// Returns [`SettingsError`] when companion files remain or deletion fails.
pub fn clear_body(settings_dir: &Path, name: &str) -> Result<(), SettingsError> {
    validate_skill_dir_name(name).map_err(invalid)?;
    let dir = skill_dir(settings_dir, name);
    if !dir.exists() {
        return Ok(());
    }
    let package = load_one(&dir, name)?;
    let companions = package
        .files
        .iter()
        .any(|file| file.relative != SKILL_BODY_FILE);
    if companions {
        return Err(invalid(format!(
            "skill `{name}` still has companion files; remove them before clearing skill.md"
        )));
    }
    // Companion files are gone. Empty directories (for example `scripts/`) may remain,
    // so remove the skill folder rather than only `skill.md` plus the top directory.
    fs::remove_dir_all(&dir).map_err(|source| SettingsError::WriteFile {
        path: dir.display().to_string(),
        source,
    })?;
    Ok(())
}

/// Deletes the whole skill directory when it exists.
///
/// # Errors
///
/// Returns [`SettingsError`] for an unsafe name or I/O failure.
pub fn remove_package_dir(settings_dir: &Path, name: &str) -> Result<(), SettingsError> {
    if validate_skill_dir_name(name).is_err() {
        return Ok(());
    }
    let dir = skill_dir(settings_dir, name);
    match fs::symlink_metadata(&dir) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SettingsError::WriteFile {
            path: dir.display().to_string(),
            source,
        }),
        Ok(meta) if meta.file_type().is_symlink() => Err(invalid(format!(
            "refusing to remove symlink skill directory `{}`",
            dir.display()
        ))),
        Ok(_) => fs::remove_dir_all(&dir).map_err(|source| SettingsError::WriteFile {
            path: dir.display().to_string(),
            source,
        }),
    }
}

/// Relative paths in the skill folder, including `skill.md`. Missing folder yields an empty list.
///
/// # Errors
///
/// Returns [`SettingsError`] when the folder exists but is not a valid package.
pub fn list_package_files(settings_dir: &Path, name: &str) -> Result<Vec<String>, SettingsError> {
    if validate_skill_dir_name(name).is_err() {
        return Ok(Vec::new());
    }
    let dir = skill_dir(settings_dir, name);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let package = load_one(&dir, name)?;
    Ok(package
        .files
        .into_iter()
        .map(|file| file.relative)
        .collect())
}

/// Reads one indexed file as UTF-8. `skill.md` is reserved for the body commands.
///
/// # Errors
///
/// Returns [`SettingsError`] when the path is unknown, not UTF-8, or unreadable.
pub fn read_package_file(
    settings_dir: &Path,
    name: &str,
    relative: &str,
) -> Result<String, SettingsError> {
    if relative == SKILL_BODY_FILE {
        return Err(invalid("use `skill body get` to read skill.md".to_string()));
    }
    let entry = require_file(settings_dir, name, relative)?;
    fs::read_to_string(&entry.canonical).map_err(|source| SettingsError::ReadFile {
        path: entry.relative,
        source,
    })
}

/// Writes a companion file inside the skill folder. `skill.md` must already exist.
///
/// # Errors
///
/// Returns [`SettingsError`] when the path escapes, the package has no `skill.md`, or I/O fails.
pub fn write_package_file(
    settings_dir: &Path,
    name: &str,
    relative: &str,
    content: &str,
) -> Result<(), SettingsError> {
    if relative == SKILL_BODY_FILE {
        return Err(invalid(
            "use `skill body set` to write skill.md".to_string(),
        ));
    }
    validate_skill_dir_name(name).map_err(invalid)?;
    validate_relative_skill_path(relative).map_err(invalid)?;
    let dir = skill_dir(settings_dir, name);
    if !dir.join(SKILL_BODY_FILE).is_file() {
        return Err(invalid(format!(
            "skill `{name}` has no skill.md; set the body before adding files"
        )));
    }
    let dest = resolve_new_file(&dir, relative)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsError::WriteFile {
            path: parent.display().to_string(),
            source,
        })?;
    }
    reject_existing_symlink(&dest)?;
    ensure_parent_inside(&dir, &dest)?;
    atomic_write(&dest, content)
}

/// Removes a companion file. Does not remove `skill.md`.
///
/// # Errors
///
/// Returns [`SettingsError`] when the path is unknown or deletion fails.
pub fn remove_package_file(
    settings_dir: &Path,
    name: &str,
    relative: &str,
) -> Result<(), SettingsError> {
    if relative == SKILL_BODY_FILE {
        return Err(invalid(
            "use `skill body clear` to remove skill.md".to_string(),
        ));
    }
    let entry = require_file(settings_dir, name, relative)?;
    fs::remove_file(&entry.canonical).map_err(|source| SettingsError::WriteFile {
        path: entry.relative,
        source,
    })
}

/// Reads `skill.md` when the skill folder exists.
///
/// # Errors
///
/// Returns [`SettingsError`] when the folder exists but is invalid.
pub fn read_body(settings_dir: &Path, name: &str) -> Result<Option<String>, SettingsError> {
    if validate_skill_dir_name(name).is_err() {
        return Ok(None);
    }
    let dir = skill_dir(settings_dir, name);
    if !dir.exists() {
        return Ok(None);
    }
    let package = load_one(&dir, name)?;
    Ok(Some(package.body))
}

fn require_file(
    settings_dir: &Path,
    name: &str,
    relative: &str,
) -> Result<SkillFileEntry, SettingsError> {
    validate_skill_dir_name(name).map_err(invalid)?;
    validate_relative_skill_path(relative).map_err(invalid)?;
    let dir = skill_dir(settings_dir, name);
    if !dir.exists() {
        return Err(invalid(format!("skill `{name}` has no folder")));
    }
    let package = load_one(&dir, name)?;
    package
        .files
        .into_iter()
        .find(|file| file.relative == relative)
        .ok_or_else(|| invalid(format!("`{relative}` is not in skill `{name}`")))
}

fn load_one(dir: &Path, name: &str) -> Result<SkillPackage, SettingsError> {
    let root = dir
        .canonicalize()
        .map_err(|source| SettingsError::ReadFile {
            path: dir.display().to_string(),
            source,
        })?;
    let mut files = Vec::new();
    walk_files(&root, &root, "", &mut files)?;
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    let body_entry = files.iter().find(|file| file.relative == SKILL_BODY_FILE);
    let Some(body_entry) = body_entry else {
        return Err(invalid(format!(
            "skill `{name}` is missing {SKILL_BODY_FILE}"
        )));
    };
    let body =
        fs::read_to_string(&body_entry.canonical).map_err(|source| SettingsError::ReadFile {
            path: body_entry.canonical.display().to_string(),
            source,
        })?;
    if body.trim().is_empty() {
        return Err(invalid(format!(
            "skill `{name}` {SKILL_BODY_FILE} must not be empty"
        )));
    }
    Ok(SkillPackage {
        name: name.to_string(),
        root,
        body,
        files,
    })
}

fn walk_files(
    root: &Path,
    dir: &Path,
    prefix: &str,
    out: &mut Vec<SkillFileEntry>,
) -> Result<(), SettingsError> {
    let entries = fs::read_dir(dir).map_err(|source| SettingsError::ReadFile {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| SettingsError::ReadFile {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|source| SettingsError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Err(invalid(format!(
                "skill file `{}` must not be a symlink",
                path.display()
            )));
        }
        let os_name = entry.file_name();
        let Some(file_name) = os_name.to_str() else {
            return Err(invalid(format!(
                "skill file `{}` is not valid UTF-8",
                path.display()
            )));
        };
        if file_name.is_empty() || file_name == "." || file_name == ".." {
            return Err(invalid(format!(
                "skill file `{}` has an unsafe name",
                path.display()
            )));
        }
        let relative = if prefix.is_empty() {
            file_name.to_string()
        } else {
            format!("{prefix}/{file_name}")
        };
        validate_relative_skill_path(&relative).map_err(invalid)?;
        if meta.is_dir() {
            walk_files(root, &path, &relative, out)?;
            continue;
        }
        if !meta.is_file() {
            return Err(invalid(format!(
                "skill entry `{}` is not a file",
                path.display()
            )));
        }
        let canonical = path
            .canonicalize()
            .map_err(|source| SettingsError::ReadFile {
                path: path.display().to_string(),
                source,
            })?;
        if !canonical.starts_with(root) {
            return Err(invalid(format!(
                "skill file `{}` escapes the skill directory",
                path.display()
            )));
        }
        out.push(SkillFileEntry {
            relative,
            canonical,
        });
    }
    Ok(())
}

fn resolve_new_file(skill_root: &Path, relative: &str) -> Result<PathBuf, SettingsError> {
    let root = skill_root
        .canonicalize()
        .map_err(|source| SettingsError::ReadFile {
            path: skill_root.display().to_string(),
            source,
        })?;
    let mut current = root.clone();
    let parts: Vec<&str> = relative.split('/').collect();
    let Some((file_name, dirs)) = parts.split_last() else {
        return Err(invalid("skill file path must name a file".to_string()));
    };
    for dir_name in dirs {
        current = current.join(dir_name);
        if current.exists() {
            let meta =
                fs::symlink_metadata(&current).map_err(|source| SettingsError::ReadFile {
                    path: current.display().to_string(),
                    source,
                })?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(invalid(format!(
                    "`{}` is not a directory inside the skill folder",
                    current.display()
                )));
            }
            current = current
                .canonicalize()
                .map_err(|source| SettingsError::ReadFile {
                    path: current.display().to_string(),
                    source,
                })?;
            if !current.starts_with(&root) {
                return Err(invalid(format!(
                    "`{}` escapes the skill directory",
                    current.display()
                )));
            }
        }
    }
    Ok(current.join(file_name))
}

fn ensure_parent_inside(skill_root: &Path, dest: &Path) -> Result<(), SettingsError> {
    let root = skill_root
        .canonicalize()
        .map_err(|source| SettingsError::ReadFile {
            path: skill_root.display().to_string(),
            source,
        })?;
    let Some(parent) = dest.parent() else {
        return Err(invalid("skill file has no parent directory".to_string()));
    };
    let parent = parent
        .canonicalize()
        .map_err(|source| SettingsError::ReadFile {
            path: parent.display().to_string(),
            source,
        })?;
    if !parent.starts_with(&root) {
        return Err(invalid(format!(
            "`{}` escapes the skill directory",
            dest.display()
        )));
    }
    Ok(())
}

fn skill_dir(settings_dir: &Path, name: &str) -> PathBuf {
    settings_dir.join(SKILL_PACKAGES_DIR).join(name)
}

fn reject_existing_symlink(path: &Path) -> Result<(), SettingsError> {
    match fs::symlink_metadata(path) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SettingsError::ReadFile {
            path: path.display().to_string(),
            source,
        }),
        Ok(meta) if meta.file_type().is_symlink() => Err(invalid(format!(
            "`{}` must not be a symlink",
            path.display()
        ))),
        Ok(_) => Ok(()),
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<(), SettingsError> {
    let temp_path = temp_path_for(path);
    fs::write(&temp_path, content).map_err(|source| SettingsError::WriteFile {
        path: path.display().to_string(),
        source,
    })?;
    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            if path.exists() {
                if let Err(source) = fs::remove_file(path) {
                    let _ = fs::remove_file(&temp_path);
                    return Err(SettingsError::WriteFile {
                        path: path.display().to_string(),
                        source,
                    });
                }
                if let Err(source) = fs::rename(&temp_path, path) {
                    let _ = fs::remove_file(&temp_path);
                    return Err(SettingsError::WriteFile {
                        path: path.display().to_string(),
                        source,
                    });
                }
                Ok(())
            } else {
                let _ = fs::remove_file(&temp_path);
                Err(SettingsError::WriteFile {
                    path: path.display().to_string(),
                    source: err,
                })
            }
        }
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    PathBuf::from(temp)
}

fn invalid(message: String) -> SettingsError {
    SettingsError::InvalidSkillPackage(message)
}

fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, body: &str, script: Option<&str>) {
        let skill = dir.join(SKILL_PACKAGES_DIR).join(name);
        fs::create_dir_all(skill.join("scripts")).expect("dirs");
        fs::write(skill.join(SKILL_BODY_FILE), body).expect("body");
        if let Some(script) = script {
            fs::write(skill.join("scripts").join("example.py"), script).expect("script");
        }
    }

    #[test]
    fn loads_skill_md_and_indexes_script_without_reading_it_into_body() {
        let dir = tempfile::tempdir().expect("temp");
        write_skill(dir.path(), "research", "# Research\n", Some("print(1)\n"));

        let packages = load_skill_packages(dir.path()).expect("packages");
        let package = packages.get("research").expect("research");
        assert_eq!(package.body, "# Research\n");
        assert!(package.body.contains("Research"));
        assert!(!package.body.contains("print"));
        let relatives: Vec<_> = package
            .files
            .iter()
            .map(|file| file.relative.as_str())
            .collect();
        assert_eq!(relatives, vec!["scripts/example.py", "skill.md"]);
    }

    #[test]
    fn missing_skills_dir_is_empty() {
        let dir = tempfile::tempdir().expect("temp");
        let packages = load_skill_packages(dir.path()).expect("packages");
        assert!(packages.is_empty());
    }

    #[test]
    fn rejects_folder_without_skill_md() {
        let dir = tempfile::tempdir().expect("temp");
        fs::create_dir_all(dir.path().join(SKILL_PACKAGES_DIR).join("research")).expect("dir");
        let err = load_skill_packages(dir.path()).expect_err("missing body");
        assert!(err.to_string().contains("skill.md"), "{err}");
    }

    #[test]
    fn rejects_symlink_inside_skill_folder_when_the_os_allows_links() {
        let dir = tempfile::tempdir().expect("temp");
        let skill = dir.path().join(SKILL_PACKAGES_DIR).join("research");
        fs::create_dir_all(&skill).expect("skill dir");
        fs::write(skill.join(SKILL_BODY_FILE), "body\n").expect("body");
        let target = dir.path().join("secret.txt");
        fs::write(&target, "secret").expect("secret");
        let link = skill.join("leak.txt");
        let created = create_symlink_file(&target, &link);
        if !created {
            return;
        }
        let err = load_skill_packages(dir.path()).expect_err("symlink");
        assert!(err.to_string().contains("symlink"), "{err}");
    }

    fn create_symlink_file(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    #[test]
    fn rejects_parent_segment_in_relative_path() {
        let err = validate_relative_skill_path("../secret").expect_err("escape");
        assert!(err.contains("inside"));
        assert!(validate_relative_skill_path("scripts/example.py").is_ok());
    }

    #[test]
    fn undeclared_package_fails_catalog_check() {
        let dir = tempfile::tempdir().expect("temp");
        write_skill(dir.path(), "extra", "body\n", None);
        let packages = load_skill_packages(dir.path()).expect("packages");
        let catalog = SkillsCatalog::parse(
            r#"skill "research" { policy: "safe" allowed_tools: ["home.read"] }"#,
        )
        .expect("dsl");
        let err = validate_packages_match_catalog(&packages, &catalog).expect_err("undeclared");
        assert!(err.to_string().contains("extra"), "{err}");
    }

    #[test]
    fn write_body_then_companion_round_trip() {
        let dir = tempfile::tempdir().expect("temp");
        write_body(dir.path(), "research", "# Steps\n").expect("body");
        write_package_file(dir.path(), "research", "scripts/example.py", "print(1)\n")
            .expect("script");
        let listed = list_package_files(dir.path(), "research").expect("list");
        assert_eq!(
            listed,
            vec!["scripts/example.py".to_string(), "skill.md".to_string()]
        );
        assert_eq!(
            read_package_file(dir.path(), "research", "scripts/example.py").expect("read"),
            "print(1)\n"
        );
        let err =
            write_package_file(dir.path(), "research", "../secret.txt", "no").expect_err("escape");
        assert!(err.to_string().contains("inside"), "{err}");
    }

    #[test]
    fn clear_body_refuses_while_companion_files_remain() {
        let dir = tempfile::tempdir().expect("temp");
        write_body(dir.path(), "research", "# Steps\n").expect("body");
        write_package_file(dir.path(), "research", "notes.md", "note\n").expect("note");
        let err = clear_body(dir.path(), "research").expect_err("companions");
        assert!(err.to_string().contains("companion"), "{err}");
        remove_package_file(dir.path(), "research", "notes.md").expect("rm");
        clear_body(dir.path(), "research").expect("clear");
        assert!(!dir
            .path()
            .join(SKILL_PACKAGES_DIR)
            .join("research")
            .exists());
    }

    #[test]
    fn rejects_unsafe_names_empty_body_and_a_skills_path_that_is_not_a_directory() {
        assert!(validate_skill_dir_name("").is_err());
        assert!(validate_skill_dir_name(" CON").is_err());
        assert!(validate_skill_dir_name("-bad").is_err());
        assert!(validate_skill_dir_name("AUX").is_err());
        assert!(validate_skill_dir_name("ok name").is_ok());
        assert!(validate_relative_skill_path("").is_err());
        assert!(validate_relative_skill_path("/abs").is_err());
        assert!(validate_relative_skill_path("foo/").is_err());
        assert!(validate_relative_skill_path("foo.").is_err());
        assert!(validate_relative_skill_path("nul").is_err());
        assert!(validate_relative_skill_path("scripts/example.py").is_ok());

        let dir = tempfile::tempdir().expect("temp");
        let err = write_body(dir.path(), "research", " \n").expect_err("empty");
        assert!(err.to_string().contains("empty"), "{err}");
        assert!(read_body(dir.path(), "research")
            .expect("missing")
            .is_none());
        assert!(read_body(dir.path(), "..").expect("unsafe").is_none());
        assert!(list_package_files(dir.path(), "..")
            .expect("unsafe")
            .is_empty());
        assert!(remove_package_dir(dir.path(), "..").is_ok());

        fs::write(dir.path().join(SKILL_PACKAGES_DIR), "not a dir").expect("file");
        let err = load_skill_packages(dir.path()).expect_err("not a dir");
        assert!(err.to_string().contains("directory"), "{err}");
        fs::remove_file(dir.path().join(SKILL_PACKAGES_DIR)).expect("rm");

        let loose = dir.path().join(SKILL_PACKAGES_DIR);
        fs::create_dir_all(&loose).expect("skills");
        fs::write(loose.join("notes.txt"), "nope").expect("loose");
        let err = load_skill_packages(dir.path()).expect_err("loose file");
        assert!(err.to_string().contains("skill directory"), "{err}");
        fs::remove_file(loose.join("notes.txt")).expect("rm loose");

        let skill = loose.join("research");
        fs::create_dir_all(&skill).expect("skill");
        fs::write(skill.join(SKILL_BODY_FILE), " \n").expect("blank");
        let err = load_skill_packages(dir.path()).expect_err("blank body");
        assert!(err.to_string().contains("empty"), "{err}");
        fs::write(skill.join(SKILL_BODY_FILE), "# Steps\n").expect("body");
        fs::write(skill.join("blob.bin"), [0xff, 0xfe]).expect("bin");
        let err = read_package_file(dir.path(), "research", SKILL_BODY_FILE).expect_err("body");
        assert!(err.to_string().contains("skill body"), "{err}");
        let err = read_package_file(dir.path(), "research", "blob.bin").expect_err("utf8");
        let message = err.to_string().to_lowercase();
        assert!(
            message.contains("utf-8") || message.contains("utf8"),
            "{err}"
        );
        let err = write_package_file(dir.path(), "research", SKILL_BODY_FILE, "x").expect_err("md");
        assert!(err.to_string().contains("skill body"), "{err}");
        let err = remove_package_file(dir.path(), "research", SKILL_BODY_FILE).expect_err("md");
        assert!(err.to_string().contains("skill body"), "{err}");
        let err = write_package_file(dir.path(), "other", "notes.md", "x").expect_err("no body");
        assert!(err.to_string().contains("skill.md"), "{err}");
        remove_package_dir(dir.path(), "research").expect("remove");
        assert!(!skill.exists());
        clear_body(dir.path(), "research").expect("clear missing");
    }
}
