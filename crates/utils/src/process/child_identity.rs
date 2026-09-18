use std::io;

/// A non-root execution identity with no supplementary groups or inheritable capabilities.
#[derive(Clone, Copy)]
pub struct LinuxChildIdentity {
    uid: u32,
    gid: u32,
}

impl LinuxChildIdentity {
    /// Rejects root and the sentinel that setresuid/setresgid interpret as "leave unchanged".
    pub fn new(uid: u32, gid: u32) -> io::Result<Self> {
        if uid == 0 || gid == 0 || uid == u32::MAX || gid == u32::MAX {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        Ok(Self { uid, gid })
    }

    /// Irreversibly removes inherited authority before an untrusted executable is loaded.
    ///
    /// # Safety
    /// Call only in an isolated child before exec. This changes credentials and descriptor flags
    /// process-wide; failure leaves partial changes and the child must exit without executing.
    /// This path uses only raw syscalls and allocation-free OS errors so it is safe after fork.
    pub unsafe fn enter_child(self) -> io::Result<()> {
        // CLOEXEC, rather than closing immediately, preserves the spawn error-reporting pipe.
        // Unsupported kernels fail closed instead of leaking a privileged inherited descriptor.
        // SAFETY: these syscalls take scalar arguments or the explicitly sized buffers below.
        unsafe {
            if libc::syscall(
                libc::SYS_close_range,
                3_u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            ) < 0
                || libc::prctl(
                    libc::PR_SET_NO_NEW_PRIVS,
                    1 as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                ) < 0
                || libc::syscall(
                    libc::SYS_setgroups,
                    0_usize,
                    std::ptr::null::<libc::gid_t>(),
                ) < 0
                || libc::syscall(libc::SYS_setresgid, self.gid, self.gid, self.gid) < 0
                || libc::syscall(libc::SYS_setresuid, self.uid, self.uid, self.uid) < 0
            {
                return Err(io::Error::last_os_error());
            }
            // Explicitly clear all capability sets, including when the parent used securebits
            // or keepcaps. Clearing inheritable/permitted also clears any ambient capabilities.
            let mut header = CapabilityHeader {
                version: 0x2008_0522,
                pid: 0,
            };
            let data = [CapabilityData {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            }; 2];
            if libc::syscall(
                libc::SYS_capset,
                &mut header as *mut CapabilityHeader,
                data.as_ptr(),
            ) < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

// Linux capability ABI v3 uses two 32-bit words for each of the three capability sets.
#[repr(C)]
struct CapabilityHeader {
    version: u32,
    pid: i32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct CapabilityData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}
