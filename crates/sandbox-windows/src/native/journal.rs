//! Crash-recovery journal for leftover AppContainer profiles.

use std::fs;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::native::profile::delete_profile_name;

fn journal_dir() -> PathBuf {
    std::env::temp_dir().join("agent-kuibysheff-sandbox-journal")
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
        let pid = std::process::id();
        fs::write(&path, format!("{profile_name}\n{pid}"))?;
        Ok(Self { path })
    }
}

impl Drop for ProfileJournalEntry {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Deletes AppContainer profiles left behind by crashed runs.
///
/// Journal entries for profiles owned by still-running processes are left alone
/// so concurrent sandbox launches do not delete each other's profiles.
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
        let Some(record) = read_journal_record(&path) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let Some(pid) = record.pid else {
            // Legacy journal entries did not record an owner PID; leave them alone
            // so concurrent runs are not torn down by mistake.
            continue;
        };
        if is_process_alive(pid) {
            continue;
        }
        delete_profile_name(&record.profile_name);
        let _ = fs::remove_file(&path);
    }
}

struct JournalRecord {
    profile_name: String,
    pid: Option<u32>,
}

fn read_journal_record(path: &Path) -> Option<JournalRecord> {
    let content = fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let profile_name = lines.next()?.trim().to_string();
    if profile_name.is_empty() {
        return None;
    }
    let pid = lines
        .next()
        .and_then(|line| line.trim().parse::<u32>().ok());
    Some(JournalRecord { profile_name, pid })
}

fn is_process_alive(pid: u32) -> bool {
    // SAFETY: OpenProcess with limited query rights; null means the PID is gone.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: handle was opened successfully and is uniquely owned here.
    unsafe {
        CloseHandle(handle);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_journal_record_parses_pid_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.profile");
        fs::write(&path, "agent.kuibysheff.sb.123\n4242").unwrap();
        let record = read_journal_record(&path).expect("record");
        assert_eq!(record.profile_name, "agent.kuibysheff.sb.123");
        assert_eq!(record.pid, Some(4242));
    }

    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
        assert!(!is_process_alive(u32::MAX));
    }
}
