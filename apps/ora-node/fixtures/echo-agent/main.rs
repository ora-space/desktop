//! Deterministic agent plugin the Node session tests run in place of Deno and a real agent CLI.
//!
//! It speaks the plugin protocol on stdio and a minimal ACP agent inside `agent/acp`. A prompt is
//! echoed back, except for these markers:
//!
//! - `[write]`, `[commit]`, `[push]`, `[fetch <ws>]` execute Git collaboration in task order;
//! - `[hold]` streams one message and completes only when the host cancels the prompt;
//! - `[request-failed]` returns an ACP error with a private diagnostic for failure-path tests;
//! - `[commit-host]` asks the host to run `git commit --allow-empty` in the session's directory through
//!   `ora/childprocess/spawn`, and completes when that process exits;
//! - `[commit-direct]` runs the same commit as its own child process, inheriting its environment;
//! - `[env]` answers with its working directory and `HOME`, showing where and as what it runs;
//! - `[large]` answers with a message larger than one Thread record may be.
//!
//! On start it appends its process id to `echo-agent.pids` in its working directory, the package
//! root, so a test can check that every plugin process it started has exited.
//!
//! Started as `<fixture> ora-git <args>` it is the `ora-git` command instead: the collaboration
//! markers run it that way, so the tests that exec it get the CLI built with them (Cargo builds
//! this package's binaries for its integration tests, never another package's).

#[cfg(target_os = "linux")]
mod agent;

/// Runs as `ora-git` when asked to, and as the agent plugin otherwise.
#[cfg(target_os = "linux")]
fn main() -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some((command, rest)) = args.split_first()
        && command == "ora-git"
    {
        return Ok(ora_git::run(rest));
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(agent::run())?;
    Ok(std::process::ExitCode::SUCCESS)
}

#[cfg(not(target_os = "linux"))]
fn main() {}
