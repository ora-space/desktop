//! Deterministic agent plugin the Node session tests run in place of Deno and a real agent CLI.
//!
//! It speaks the plugin protocol on stdio and a minimal ACP agent inside `agent/acp`. A prompt is
//! echoed back, except for these markers:
//!
//! - `[hold]` streams one message and completes only when the host cancels the prompt;
//! - `[request-failed]` returns an ACP error with a private diagnostic for failure-path tests;
//! - `[commit]` asks the host to run `git commit --allow-empty` in the session's directory through
//!   `ora/childprocess/spawn`, and completes when that process exits;
//! - `[commit-direct]` runs the same commit as its own child process, inheriting its environment;
//! - `[env]` answers with its working directory and `HOME`, showing where and as what it runs;
//! - `[large]` answers with a message larger than one Thread record may be.
//!
//! On start it appends its process id to `echo-agent.pids` in its working directory, the package
//! root, so a test can check that every plugin process it started has exited.

#[cfg(target_os = "linux")]
mod agent;

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agent::run().await
}

#[cfg(not(target_os = "linux"))]
fn main() {}
