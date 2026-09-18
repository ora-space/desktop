use std::process::Command;

#[cfg(target_os = "linux")]
mod linux_detached;
#[cfg(target_os = "linux")]
pub use linux_detached::configure_linux_detached_child;

#[cfg(unix)]
mod pipe_capture;
#[cfg(unix)]
pub use pipe_capture::{PipeCapture, PipeRead, PipeReadState};

#[cfg(target_os = "linux")]
mod child_identity;
#[cfg(target_os = "linux")]
pub use child_identity::LinuxChildIdentity;
#[cfg(target_os = "linux")]
mod linux_process;
#[cfg(target_os = "linux")]
pub use linux_process::{
    LinuxPidFd, LinuxProcessStat, ProcessSignal, linux_process, linux_process_snapshot,
};

/// Configures a background child so it does not surface a console window on Windows.
pub fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        // GUI applications still need pipes and exit statuses from console-subsystem tools, but
        // those implementation details must not create terminal windows beside the product UI.
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Hides a Windows child while also placing it in an independent process group.
#[cfg(windows)]
pub fn hide_console_window_in_new_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

    // `creation_flags` replaces the configured bitset, so both requirements belong in one call.
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}
