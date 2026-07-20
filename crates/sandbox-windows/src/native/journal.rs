//! Crash-recovery journal for leftover AppContainer profiles.

use std::fs;
use std::path::PathBuf;

use crate::native::profile::delete_profile_name;

fn journal_dir() -> PathBuf {
    std::env::temp_dir().join("agent-kuibyshev-sandbox-journal")
}

/// Records a live profile name so a future process can delete it after a crash.
pub struct ProfileJournalEntry {
    path: PathBuf,
}

impl ProfileJournalEntry {
    pub fn create(profile_name: &str) -> std::io::Result<Self> {
        let dir = journal_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{profile_name}.profile"));
        fs::write(&path, profile_name)?;
        Ok(Self { path })
    }
}

impl Drop for ProfileJournalEntry {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Deletes stale AppContainer profiles recorded by previous crashed runs.
pub fn reclaim_stale_profiles() {
    let dir = journal_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("profile") {
            continue;
        }
        if let Ok(name) = fs::read_to_string(&path) {
            let name = name.trim();
            if !name.is_empty() {
                delete_profile_name(name);
            }
        }
        let _ = fs::remove_file(&path);
    }
}
