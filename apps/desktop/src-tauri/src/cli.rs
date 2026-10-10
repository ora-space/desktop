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
/// neither; the message is discarded and the process still exits. Help, version,
/// and the already-running notice share this path, because a GUI-subsystem process
/// also has no stderr for `eprintln!`.
pub(crate) fn write_parent_console(text: &str) {
    #[cfg(windows)]
    {
        if has_stdout_handle() {
            write_stdout_line(text);
            return;
        }
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 };
        if attached && let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$")
        {
            use std::io::Write;
            let _ = writeln!(console, "{text}");
            return;
        }
    }
    write_stdout_line(text);
}

/// Writes one line without panicking when the handle is missing or closed.
///
/// `println!` panics on a failed write. On a GUI-subsystem launch that panic
/// would skip the `process::exit` or Tauri exit that follows help, version, and
/// the already-running notice, so the caller must get control back.
fn write_stdout_line(text: &str) {
    let mut stdout = std::io::stdout().lock();
    write_line(&mut stdout, text);
}

/// Ignores a failed write. The message is optional; leaving the process is not.
fn write_line(writer: &mut impl std::io::Write, text: &str) {
    let _ = writeln!(writer, "{text}");
}

/// Whether this process was given a standard-output handle at creation.
///
/// `STD_OUTPUT_HANDLE` is `(DWORD)-11`, not `11`. The `windows-sys` constant is
/// that value; a hand-written `11` makes `GetStdHandle` fail and would send every
/// launch down the parent-console path. A GUI-subsystem launch without redirection
/// gets either `NULL` or `INVALID_HANDLE_VALUE`.
#[cfg(windows)]
fn has_stdout_handle() -> bool {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE};
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

#[cfg(test)]
mod tests {
    use super::{CliAction, classify_args, write_line};
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

    /// A broken stdout must return. `println!` would panic and skip the exit.
    #[test]
    fn a_closed_output_does_not_panic() {
        struct Closed;
        impl std::io::Write for Closed {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ))
            }
        }
        write_line(&mut Closed, "ora-desktop already-running notice");
    }
}
