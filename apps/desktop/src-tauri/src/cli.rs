//! Flags that must finish before Tauri, SQLite, or the plugin runtime start.
//!
//! A graphical launch has no command to run. `--help` and `--version` exist so a
//! parent process can ask a question and leave. Anything else, including an
//! unrecognized flag, is a normal launch: rejecting unknown arguments would also
//! reject a future file or protocol handoff from the operating system.

use std::ffi::OsStr;

/// What `main` should do with the process arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliAction {
    /// Build the window and open the desktop backend.
    Run,
    /// Print usage and exit.
    Help,
    /// Print the package version and exit.
    Version,
}

/// Classifies the first user argument. `argv[0]`, the program path, is ignored.
pub(crate) fn classify_args<I, S>(args: I) -> CliAction
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    match args.next() {
        Some(flag)
            if flag.as_ref() == OsStr::new("--help") || flag.as_ref() == OsStr::new("-h") =>
        {
            CliAction::Help
        }
        Some(flag)
            if flag.as_ref() == OsStr::new("--version") || flag.as_ref() == OsStr::new("-V") =>
        {
            CliAction::Version
        }
        _ => CliAction::Run,
    }
}

/// Prints usage for a parent terminal and returns.
pub(crate) fn print_help() {
    write_parent_console(&format!(
        "\
Ora Desktop {version}

Usage: ora-desktop [--help | --version]

Starts the Ora desktop application. This is a graphical program.

  --help, -h       Show this help and exit without opening the database
  --version, -V    Print the version and exit without opening the database

Starting the application again while it is already running focuses the
existing window and exits. That second process does not open the database
and does not interrupt workflow runs that are still in progress.
",
        version = env!("CARGO_PKG_VERSION"),
    ));
}

/// Prints the Cargo package version for a parent terminal and returns.
pub(crate) fn print_version() {
    write_parent_console(&format!("ora-desktop {}", env!("CARGO_PKG_VERSION")));
}

/// Writes one message where the caller can read it.
///
/// Release builds use the Windows GUI subsystem, so the process has no console
/// of its own. When the launcher already provided a stdout — an inherited console
/// buffer, a pipe, or a redirected file — that handle is where the answer belongs:
/// attaching the parent console would bypass it and leave `ora-desktop --version |
/// grep` reading an empty pipe. Only a launch without any stdout falls back to
/// attaching the parent console and writing `CONOUT$`. A double-clicked launch has
/// neither; the message is discarded and the process still exits.
fn write_parent_console(text: &str) {
    #[cfg(windows)]
    {
        if has_stdout_handle() {
            println!("{text}");
            return;
        }
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
        unsafe extern "system" {
            fn AttachConsole(dw_process_id: u32) -> i32;
        }
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 };
        if attached && let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$")
        {
            use std::io::Write;
            let _ = writeln!(console, "{text}");
            return;
        }
    }
    println!("{text}");
}

/// Whether this process was given a standard-output handle at creation.
///
/// `STD_OUTPUT_HANDLE` is `(DWORD)-11`, not `11`; passing the wrong constant makes
/// `GetStdHandle` fail and would send every launch down the parent-console path. A
/// GUI-subsystem launch without redirection gets either `NULL` or the invalid-handle
/// sentinel, and `println!` is a safe no-op in that case, so callers can always fall
/// back to it.
#[cfg(windows)]
fn has_stdout_handle() -> bool {
    use std::ffi::c_void;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> *mut c_void;
    }
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) } as usize;
    handle != 0 && handle != usize::MAX
}

#[cfg(test)]
mod tests {
    use super::{CliAction, classify_args};
    use pretty_assertions::assert_eq;

    #[test]
    fn help_and_version_exit_before_a_normal_launch() {
        assert_eq!(classify_args(["ora-desktop", "--help"]), CliAction::Help);
        assert_eq!(classify_args(["ora-desktop", "-h"]), CliAction::Help);
        assert_eq!(
            classify_args(["ora-desktop", "--version"]),
            CliAction::Version
        );
        assert_eq!(classify_args(["ora-desktop", "-V"]), CliAction::Version);
        assert_eq!(classify_args(["ora-desktop"]), CliAction::Run);
        assert_eq!(
            classify_args(["ora-desktop", "--not-a-known-flag"]),
            CliAction::Run
        );
    }
}
