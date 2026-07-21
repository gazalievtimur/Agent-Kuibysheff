//! Minimal seccomp denylist for sandbox escape primitives.

use libc::{
    sock_filter, sock_fprog, SECCOMP_MODE_FILTER, SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO,
    SECCOMP_RET_KILL_PROCESS,
};

use crate::error::{SandboxLinuxError, SandboxStage};
use crate::native::util::errno_err;

const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

const SECCOMP_DATA_NR_OFFSET: u32 = 0;
const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;

fn bpf(code: u16, jt: u8, jf: u8, k: u32) -> sock_filter {
    sock_filter { code, jt, jf, k }
}

/// Installs a fail-closed denylist (x86_64). Unknown arches refuse to proceed.
pub fn install_denylist() -> Result<(), SandboxLinuxError> {
    let denied: &[i64] = &[
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_bpf,
        libc::SYS_userfaultfd,
        libc::SYS_perf_event_open,
        libc::SYS_kexec_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_syslog,
        libc::SYS_clone3,
        libc::SYS_open_tree,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_mount_setattr,
    ];

    let mut filters: Vec<sock_filter> = Vec::with_capacity(8 + denied.len() * 2);
    filters.push(bpf(
        BPF_LD | BPF_W | BPF_ABS,
        0,
        0,
        SECCOMP_DATA_ARCH_OFFSET,
    ));
    filters.push(bpf(BPF_JMP | BPF_JEQ | BPF_K, 1, 0, AUDIT_ARCH_X86_64));
    filters.push(bpf(BPF_RET | BPF_K, 0, 0, SECCOMP_RET_KILL_PROCESS));
    filters.push(bpf(BPF_LD | BPF_W | BPF_ABS, 0, 0, SECCOMP_DATA_NR_OFFSET));

    for sys in denied {
        // equal → fall through to RET; not equal → skip RET
        filters.push(bpf(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, *sys as u32));
        let action = if *sys == libc::SYS_clone3 {
            SECCOMP_RET_ERRNO | (libc::ENOSYS as u32)
        } else {
            SECCOMP_RET_ERRNO | (libc::EPERM as u32)
        };
        filters.push(bpf(BPF_RET | BPF_K, 0, 0, action));
    }
    filters.push(bpf(BPF_RET | BPF_K, 0, 0, SECCOMP_RET_ALLOW));

    let mut prog = sock_fprog {
        len: filters.len() as u16,
        filter: filters.as_mut_ptr(),
    };

    // SAFETY: kernel copies the BPF program during PR_SET_SECCOMP.
    let rc = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            std::ptr::from_mut(&mut prog),
        )
    };
    drop(filters);
    if rc != 0 {
        return Err(errno_err(SandboxStage::Seccomp, "PR_SET_SECCOMP"));
    }
    Ok(())
}
