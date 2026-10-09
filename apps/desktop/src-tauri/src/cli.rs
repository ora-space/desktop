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

/// Writes one message where a parent terminal can read it.
///
/// Release builds use the Windows GUI subsystem, so the process has no console
/// of its own. Attaching the parent console is what makes `ora-desktop --version`
/// visible to the program that spawned it. A double-clicked launch has no parent
/// console; the message is discarded and the process still exits.
fn write_parent_console(text: &str) {
    #[cfg(windows)]
    {
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
