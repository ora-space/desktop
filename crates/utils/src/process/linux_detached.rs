use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Starts an independent Linux session and closes unintended descriptors at exec.
///
/// Explicit standard-stream mappings remain available. Unsupported close_range kernels fail
/// before exec; the spawn error pipe stays open until exec so failures can still be reported.
pub fn configure_linux_detached_child(command: &mut Command) {
    // SAFETY: the child hook performs only allocation-free system calls and OS error construction.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0
                || libc::syscall(
                    libc::SYS_close_range,
                    3_u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}
