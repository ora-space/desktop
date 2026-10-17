//! `ora-git`: asks the Node to fetch from or push to the Git Server for this Agent session.
//!
//! Exit codes and the `ora-git: <code>: <hint>` line on stderr are stable (Node clone ADR D5);
//! progress goes to stderr and the result to stdout.

/// Runs the command line through the library so tests can run the same entry point.
fn main() -> std::process::ExitCode {
    ora_git::run(&std::env::args().skip(1).collect::<Vec<_>>())
}
