//! Suspended AppContainer process launch, token checks, wait, and output capture.

use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, FALSE, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE, TRUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::NetworkManagement::WindowsFirewall::NetworkIsolationGetAppContainerConfig;
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, TokenAppContainerSid, TokenCapabilities, TokenIsAppContainer,
    PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
    TOKEN_APPCONTAINER_INFORMATION, TOKEN_GROUPS, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapFree};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcessToken, ResumeThread, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::acl::{AclJournal, ACCESS_EXECUTE, ACCESS_READ, ACCESS_WRITE};
use crate::native::folder::appcontainer_folder;
use crate::native::job::JobObject;
use crate::native::pipes::PipePair;
use crate::native::profile::AppContainerProfile;
use crate::native::util::{
    build_command_line, build_environment_block, path_to_wide_null, setup_last, to_wide_null,
};
use crate::request::{SandboxLaunchRequest, SandboxLaunchResult};
use crate::OwnedHandle;

struct AttrList {
    buffer: Vec<u8>,
}

impl AttrList {
    fn new(attribute_count: u32) -> Result<Self, SandboxWindowsError> {
        let mut size = 0usize;
        // SAFETY: size-query call with a null list pointer is required by the API.
        let _ = unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), attribute_count, 0, &mut size)
        };
        if size == 0 {
            return Err(setup_last(
                SandboxStage::CreateProcess,
                "InitializeProcThreadAttributeList size query",
            ));
        }
        let mut buffer = vec![0u8; size];
        // SAFETY: buffer is large enough for `attribute_count` attributes.
        let ok = unsafe {
            InitializeProcThreadAttributeList(
                buffer.as_mut_ptr().cast(),
                attribute_count,
                0,
                &mut size,
            )
        };
        if ok == 0 {
            return Err(setup_last(
                SandboxStage::CreateProcess,
                "InitializeProcThreadAttributeList",
            ));
        }
        Ok(Self { buffer })
    }

    fn as_mut_ptr(
        &mut self,
    ) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_mut_ptr().cast()
    }
}

impl Drop for AttrList {
    fn drop(&mut self) {
        // SAFETY: list was initialized by InitializeProcThreadAttributeList.
        unsafe {
            DeleteProcThreadAttributeList(self.buffer.as_mut_ptr().cast());
        }
    }
}

/// Applies filesystem ACL grants required by `request`.
pub fn apply_access_grants(
    journal: &mut AclJournal,
    package_sid: PSID,
    request: &SandboxLaunchRequest,
) -> Result<(), SandboxWindowsError> {
    for path in &request.home_write {
        journal.grant_path(path, package_sid, ACCESS_WRITE, true)?;
    }
    for path in &request.home_read {
        journal.grant_path(path, package_sid, ACCESS_READ, true)?;
    }
    for path in &request.runtime_read_roots {
        journal.grant_path(path, package_sid, ACCESS_EXECUTE, true)?;
    }
    journal.grant_path(&request.executable, package_sid, ACCESS_EXECUTE, true)?;
    if let Some(parent) = request.executable.parent() {
        if parent.exists() {
            journal.grant_path(parent, package_sid, ACCESS_EXECUTE, true)?;
        }
    }
    journal.grant_path(&request.cwd, package_sid, ACCESS_EXECUTE, true)?;
    Ok(())
}

/// Ensures the package SID is not present in the global loopback exemption list.
pub fn assert_no_loopback_exemption(package_sid: PSID) -> Result<(), SandboxWindowsError> {
    let mut count = 0u32;
    let mut list: *mut SID_AND_ATTRIBUTES = ptr::null_mut();
    // SAFETY: NetworkIsolationGetAppContainerConfig allocates `list` on success.
    let status = unsafe { NetworkIsolationGetAppContainerConfig(&mut count, &mut list) };
    if status != 0 {
        return Err(SandboxWindowsError::unavailable(format!(
            "NetworkIsolationGetAppContainerConfig failed status={status}"
        )));
    }
    let result = (|| {
        if list.is_null() || count == 0 {
            return Ok(());
        }
        for i in 0..count as usize {
            // SAFETY: `list` has `count` SID_AND_ATTRIBUTES entries.
            let entry = unsafe { &*list.add(i) };
            // SAFETY: EqualSid compares two valid SIDs.
            let same = unsafe { EqualSid(package_sid, entry.Sid) };
            if same != FALSE {
                return Err(SandboxWindowsError::setup(
                    SandboxStage::TokenCheck,
                    "AppContainer SID is loopback-exempt",
                ));
            }
        }
        Ok(())
    })();
    if !list.is_null() {
        // SAFETY: list was allocated for NetworkIsolationGetAppContainerConfig.
        unsafe {
            HeapFree(GetProcessHeap(), 0, list.cast());
        }
    }
    result
}

fn open_nul_inheritable() -> Result<OwnedHandle, SandboxWindowsError> {
    let nul = to_wide_null(r"\\.\NUL");
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: TRUE,
    };
    // SAFETY: open NUL as a readable, inheritable stdin for the child.
    let raw = unsafe {
        CreateFileW(
            nul.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            &mut sa,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(setup_last(SandboxStage::Pipes, "CreateFileW(NUL)"));
    }
    // SAFETY: uniquely owned NUL handle.
    Ok(unsafe { OwnedHandle::from_raw(raw) })
}

fn read_pipe_bounded(handle: OwnedHandle, max_chars: usize) -> (String, bool) {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut read = 0u32;
        // SAFETY: ReadFile on an owned pipe read end.
        let ok = unsafe {
            ReadFile(
                handle.as_raw(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..read as usize]);
        // Stop early once we clearly exceed the char budget (UTF-8 upper bound).
        if bytes.len() > max_chars.saturating_mul(4).saturating_add(64) {
            break;
        }
    }
    let lossy = String::from_utf8_lossy(&bytes);
    if lossy.chars().count() > max_chars {
        let truncated: String = lossy.chars().take(max_chars).collect();
        (truncated, true)
    } else {
        (lossy.into_owned(), false)
    }
}

struct LaunchedProcess {
    process: OwnedHandle,
    thread: OwnedHandle,
}

fn create_suspended_appcontainer(
    request: &SandboxLaunchRequest,
    profile: &AppContainerProfile,
    job: &JobObject,
    stdin: &OwnedHandle,
    stdout_writer: HANDLE,
    stderr_writer: HANDLE,
) -> Result<LaunchedProcess, SandboxWindowsError> {
    let mut attr = AttrList::new(2)?;
    let mut caps = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid(),
        Capabilities: ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    // SAFETY: attribute list owns the SECURITY_CAPABILITIES pointer for CreateProcess lifetime.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            attr.as_mut_ptr(),
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            ptr::from_mut(&mut caps).cast(),
            mem::size_of::<SECURITY_CAPABILITIES>(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if ok == 0 {
        return Err(setup_last(
            SandboxStage::CreateProcess,
            "UpdateProcThreadAttribute(SECURITY_CAPABILITIES)",
        ));
    }

    let mut handles = [stdin.as_raw(), stdout_writer, stderr_writer];
    // SAFETY: HANDLE_LIST references inheritable handles valid through CreateProcessW.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            attr.as_mut_ptr(),
            0,
            windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_mut_ptr().cast(),
            mem::size_of_val(&handles),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if ok == 0 {
        return Err(setup_last(
            SandboxStage::CreateProcess,
            "UpdateProcThreadAttribute(HANDLE_LIST)",
        ));
    }

    let mut siex: STARTUPINFOEXW = unsafe { mem::zeroed() };
    siex.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
    siex.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    siex.StartupInfo.hStdInput = stdin.as_raw();
    siex.StartupInfo.hStdOutput = stdout_writer;
    siex.StartupInfo.hStdError = stderr_writer;
    siex.lpAttributeList = attr.as_mut_ptr();

    let app_name = path_to_wide_null(&request.executable);
    let mut cmd_line = build_command_line(&request.executable, &request.argv)?;
    let runtime_env = minimal_runtime_env(request, &request.cwd)?;
    let env = build_environment_block(&runtime_env)?;
    let cwd = path_to_wide_null(&request.cwd);

    let mut pi: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;
    // SAFETY: all buffers are NUL-terminated; siex attribute list is initialized.
    let ok = unsafe {
        CreateProcessW(
            app_name.as_ptr(),
            cmd_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            TRUE,
            flags,
            env.as_ptr().cast(),
            cwd.as_ptr(),
            ptr::from_ref(&siex.StartupInfo),
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(setup_last(SandboxStage::CreateProcess, "CreateProcessW"));
    }

    // SAFETY: CreateProcessW returned unique process/thread handles.
    let process = unsafe { OwnedHandle::from_raw(pi.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw(pi.hThread) };

    job.assign(process.as_raw())?;
    verify_suspended_token(process.as_raw(), profile.sid(), job)?;
    Ok(LaunchedProcess { process, thread })
}

fn verify_suspended_token(
    process: HANDLE,
    expected_sid: PSID,
    job: &JobObject,
) -> Result<(), SandboxWindowsError> {
    let mut token_raw: HANDLE = ptr::null_mut();
    // SAFETY: OpenProcessToken on a valid suspended process.
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token_raw) };
    if ok == 0 {
        return Err(setup_last(SandboxStage::TokenCheck, "OpenProcessToken"));
    }
    // SAFETY: token handle uniquely owned.
    let token = unsafe { OwnedHandle::from_raw(token_raw) };

    let mut is_ac: u32 = 0;
    let mut ret_len = 0u32;
    // SAFETY: TokenIsAppContainer writes a DWORD into is_ac.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw(),
            TokenIsAppContainer,
            ptr::from_mut(&mut is_ac).cast(),
            mem::size_of::<u32>() as u32,
            &mut ret_len,
        )
    };
    if ok == 0 || is_ac == 0 {
        return Err(SandboxWindowsError::setup(
            SandboxStage::TokenCheck,
            "process token is not an AppContainer",
        ));
    }

    let mut sid_buf = vec![0u8; 256];
    ret_len = 0;
    // SAFETY: TokenAppContainerSid fills TOKEN_APPCONTAINER_INFORMATION into sid_buf.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw(),
            TokenAppContainerSid,
            sid_buf.as_mut_ptr().cast(),
            sid_buf.len() as u32,
            &mut ret_len,
        )
    };
    if ok == 0 {
        return Err(setup_last(
            SandboxStage::TokenCheck,
            "GetTokenInformation(TokenAppContainerSid)",
        ));
    }
    // SAFETY: buffer holds TOKEN_APPCONTAINER_INFORMATION on success.
    let info = unsafe { &*sid_buf.as_ptr().cast::<TOKEN_APPCONTAINER_INFORMATION>() };
    // SAFETY: compare expected package SID with token AppContainer SID.
    let same = unsafe { EqualSid(expected_sid, info.TokenAppContainer) };
    if same == FALSE {
        return Err(SandboxWindowsError::setup(
            SandboxStage::TokenCheck,
            "AppContainer SID mismatch",
        ));
    }

    let mut cap_len = 0u32;
    // SAFETY: size probe for TokenCapabilities.
    let _ = unsafe {
        GetTokenInformation(
            token.as_raw(),
            TokenCapabilities,
            ptr::null_mut(),
            0,
            &mut cap_len,
        )
    };
    if cap_len > 0 {
        let mut cap_buf = vec![0u8; cap_len as usize];
        // SAFETY: retrieve capability groups.
        let ok = unsafe {
            GetTokenInformation(
                token.as_raw(),
                TokenCapabilities,
                cap_buf.as_mut_ptr().cast(),
                cap_len,
                &mut cap_len,
            )
        };
        if ok == 0 {
            return Err(setup_last(
                SandboxStage::TokenCheck,
                "GetTokenInformation(TokenCapabilities)",
            ));
        }
        // SAFETY: buffer starts with TOKEN_GROUPS.
        let groups = unsafe { &*cap_buf.as_ptr().cast::<TOKEN_GROUPS>() };
        if groups.GroupCount != 0 {
            return Err(SandboxWindowsError::setup(
                SandboxStage::TokenCheck,
                "AppContainer token has non-empty capabilities",
            ));
        }
    }

    assert_no_loopback_exemption(expected_sid)?;

    let mut in_job = FALSE;
    // SAFETY: IsProcessInJob checks membership in our job object.
    let ok = unsafe { IsProcessInJob(process, job.as_raw(), &mut in_job) };
    if ok == 0 || in_job == FALSE {
        return Err(SandboxWindowsError::setup(
            SandboxStage::TokenCheck,
            "process is not in the sandbox job",
        ));
    }
    Ok(())
}

/// Full AppContainer launch: grants → stage under profile folder → create → wait.
pub fn run_sandboxed(
    request: &SandboxLaunchRequest,
    profile: &AppContainerProfile,
    journal: &mut AclJournal,
) -> Result<SandboxLaunchResult, SandboxWindowsError> {
    apply_access_grants(journal, profile.sid(), request)?;
    assert_no_loopback_exemption(profile.sid())?;

    let staged = stage_in_profile_folder(request, profile.sid())?;
    let staged_request = SandboxLaunchRequest {
        executable: staged.executable,
        argv: request.argv.clone(),
        cwd: staged.cwd,
        env: request.env.clone(),
        home_read: request.home_read.clone(),
        home_write: request.home_write.clone(),
        runtime_read_roots: request.runtime_read_roots.clone(),
        deadline: request.deadline,
        max_output_chars: request.max_output_chars,
        allow_children: request.allow_children,
    };

    let job = JobObject::create(request.allow_children, None)?;
    let stdout = PipePair::create_inheritable()?;
    let stderr = PipePair::create_inheritable()?;
    let stdin = open_nul_inheritable()?;

    let launched = create_suspended_appcontainer(
        &staged_request,
        profile,
        &job,
        &stdin,
        stdout.writer.as_raw(),
        stderr.writer.as_raw(),
    )?;

    // Close write ends in the parent before resume so a quick child exit yields EOF.
    drop(stdout.writer);
    drop(stderr.writer);
    drop(stdin);

    // HANDLEs are opaque kernel refs; transfer ownership across threads via usize.
    let stdout_bits = stdout.reader.into_raw() as usize;
    let stderr_bits = stderr.reader.into_raw() as usize;
    let max_chars = request.max_output_chars;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        // SAFETY: bits uniquely own the pipe read handle from into_raw.
        let handle = unsafe { OwnedHandle::from_raw(stdout_bits as HANDLE) };
        let _ = stdout_tx.send(read_pipe_bounded(handle, max_chars));
    });
    thread::spawn(move || {
        // SAFETY: bits uniquely own the pipe read handle from into_raw.
        let handle = unsafe { OwnedHandle::from_raw(stderr_bits as HANDLE) };
        let _ = stderr_tx.send(read_pipe_bounded(handle, max_chars));
    });

    // SAFETY: ResumeThread on the primary suspended thread.
    let resumed = unsafe { ResumeThread(launched.thread.as_raw()) };
    if resumed == u32::MAX {
        job.terminate(1);
        return Err(setup_last(SandboxStage::Resume, "ResumeThread"));
    }

    let timeout_ms = duration_to_millis(request.deadline);
    // SAFETY: wait on the process handle.
    let wait = unsafe { WaitForSingleObject(launched.process.as_raw(), timeout_ms) };
    let mut timed_out = false;
    if wait == WAIT_TIMEOUT {
        timed_out = true;
        // SAFETY: force-kill the primary process then the whole job tree.
        let _ = unsafe { TerminateProcess(launched.process.as_raw(), 1) };
        job.terminate(1);
        let _ = unsafe { WaitForSingleObject(launched.process.as_raw(), 5_000) };
    } else if wait != WAIT_OBJECT_0 {
        let _ = unsafe { TerminateProcess(launched.process.as_raw(), 1) };
        job.terminate(1);
        return Err(setup_last(SandboxStage::Resume, "WaitForSingleObject"));
    }

    let mut exit_code = 1u32;
    // SAFETY: process has exited or been terminated.
    let _ = unsafe { GetExitCodeProcess(launched.process.as_raw(), &mut exit_code) };

    // Drop process handles before joining readers so pipe write ends can close.
    drop(launched);

    let (stdout_text, stdout_truncated) =
        recv_output(stdout_rx, Duration::from_secs(3)).unwrap_or_else(|| (String::new(), true));
    let (stderr_text, stderr_truncated) =
        recv_output(stderr_rx, Duration::from_secs(3)).unwrap_or_else(|| (String::new(), true));

    Ok(SandboxLaunchResult {
        stdout: stdout_text,
        stderr: stderr_text,
        stdout_truncated,
        stderr_truncated,
        exit_code: Some(exit_code as i32),
        timed_out,
    })
}

fn recv_output(rx: mpsc::Receiver<(String, bool)>, timeout: Duration) -> Option<(String, bool)> {
    rx.recv_timeout(timeout).ok()
}

fn duration_to_millis(deadline: Duration) -> u32 {
    u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX)
}

fn minimal_runtime_env(
    request: &SandboxLaunchRequest,
    profile_cwd: &Path,
) -> Result<std::collections::BTreeMap<String, String>, SandboxWindowsError> {
    let mut env = request.env.clone();
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let system_root_s = system_root.to_string_lossy().into_owned();
    let profile_s = profile_cwd.to_string_lossy().into_owned();
    let drive = if system_root_s.len() >= 2 {
        system_root_s[..2].to_string()
    } else {
        "C:".to_string()
    };

    env.entry("SystemRoot".to_string())
        .or_insert_with(|| system_root_s.clone());
    env.entry("windir".to_string())
        .or_insert_with(|| system_root_s.clone());
    env.entry("SystemDrive".to_string()).or_insert(drive);
    env.entry("PATH".to_string())
        .or_insert_with(|| system_root.join("System32").to_string_lossy().into_owned());
    // AppContainer process init expects these; point them at the package profile folder.
    env.entry("USERPROFILE".to_string())
        .or_insert_with(|| profile_s.clone());
    env.entry("HOMEPATH".to_string())
        .or_insert_with(|| profile_s.clone());
    env.entry("LOCALAPPDATA".to_string())
        .or_insert_with(|| profile_s.clone());
    env.entry("APPDATA".to_string())
        .or_insert_with(|| profile_s.clone());
    env.entry("TEMP".to_string())
        .or_insert_with(|| profile_s.clone());
    env.entry("TMP".to_string())
        .or_insert_with(|| profile_s.clone());
    Ok(env)
}

struct StagedLaunch {
    executable: PathBuf,
    cwd: PathBuf,
}

/// Copies the executable into the AppContainer profile folder so path traversal to
/// host temp directories is not required for process creation.
fn stage_in_profile_folder(
    request: &SandboxLaunchRequest,
    package_sid: PSID,
) -> Result<StagedLaunch, SandboxWindowsError> {
    let folder = appcontainer_folder(package_sid)?;
    std::fs::create_dir_all(&folder).map_err(|err| {
        SandboxWindowsError::setup(
            SandboxStage::Profile,
            format!("create profile folder {}: {err}", folder.display()),
        )
    })?;

    let file_name =
        request
            .executable
            .file_name()
            .ok_or_else(|| SandboxWindowsError::PolicyDenied {
                reason: "executable has no file name".to_string(),
            })?;
    let staged_exe = folder.join(file_name);
    std::fs::copy(&request.executable, &staged_exe).map_err(|err| {
        SandboxWindowsError::setup(
            SandboxStage::AclGrant,
            format!(
                "stage executable {} -> {}: {err}",
                request.executable.display(),
                staged_exe.display()
            ),
        )
    })?;

    Ok(StagedLaunch {
        executable: staged_exe,
        cwd: folder,
    })
}

#[allow(dead_code)]
fn _keep_close_handle_import() {
    let _ = (CloseHandle, Path::new("."));
}
