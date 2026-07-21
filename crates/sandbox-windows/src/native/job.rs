//! Job Object RAII (KILL_ON_JOB_CLOSE + optional process/memory limits).

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use crate::error::{SandboxStage, SandboxWindowsError};
use crate::native::util::setup_last;
use crate::OwnedHandle;

/// Owned Job Object configured to kill children on close.
pub struct JobObject {
    handle: OwnedHandle,
}

impl JobObject {
    /// Creates a job with `KILL_ON_JOB_CLOSE` (breakaway is never enabled).
    pub fn create(
        allow_children: bool,
        memory_limit_bytes: Option<u64>,
    ) -> Result<Self, SandboxWindowsError> {
        // SAFETY: CreateJobObjectW returns a new job handle or NULL.
        let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(setup_last(SandboxStage::Job, "CreateJobObjectW"));
        }
        // SAFETY: `raw` is a uniquely owned job handle from CreateJobObjectW.
        let handle = unsafe { OwnedHandle::from_raw(raw) };

        // SAFETY: JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a Win32 POD struct; all-bits-zero is a
        // valid representation before we set LimitFlags and related fields.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if !allow_children {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            info.BasicLimitInformation.ActiveProcessLimit = 1;
        }
        if let Some(bytes) = memory_limit_bytes {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            info.JobMemoryLimit = usize::try_from(bytes).unwrap_or(usize::MAX);
        }

        // SAFETY: SetInformationJobObject applies `info` to the valid job handle.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw(),
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<core::ffi::c_void>(),
                mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(setup_last(SandboxStage::Job, "SetInformationJobObject"));
        }

        Ok(Self { handle })
    }

    #[must_use]
    pub fn as_raw(&self) -> HANDLE {
        self.handle.as_raw()
    }

    /// Assigns a process to this job.
    pub fn assign(&self, process: HANDLE) -> Result<(), SandboxWindowsError> {
        // SAFETY: both handles are valid for AssignProcessToJobObject.
        let ok = unsafe { AssignProcessToJobObject(self.handle.as_raw(), process) };
        if ok == 0 {
            return Err(setup_last(SandboxStage::Job, "AssignProcessToJobObject"));
        }
        Ok(())
    }

    /// Terminates every process in the job.
    pub fn terminate(&self, exit_code: u32) {
        // SAFETY: TerminateJobObject on a valid job handle.
        let _ = unsafe { TerminateJobObject(self.handle.as_raw(), exit_code) };
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        self.terminate(1);
    }
}
