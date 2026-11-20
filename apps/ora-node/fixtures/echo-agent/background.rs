//! Tool background jobs the way real Agent tools leave them behind.
//!
//! OpenCode's bash tool and Claude's Bash run each command in a shell that starts its own session.
//! A `cmd &` in that shell outlives it: init adopts the job, which is in no process group the
//! Node created and, like a job Bun started, holds none of the Node's descriptors. The markers
//! start such a job in the session's directory:
//!
//! - `[background <ms>]` writes `background-leak` there after `<ms>` milliseconds;
//! - `[background-hidden <ms>]` does the same from a non-dumpable process, whose environment no
//!   other process of the user can read, under a shell that waits for it.
//!
//! Each job first records its PID in `background.pid`, so a test can pin it before checking that
//! it never wrote.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Subcommand of the fixture executable that is the job itself.
pub(super) const JOB_COMMAND: &str = "background-job";

/// Starts the job every background marker of `prompt` asks for.
pub(super) fn start(cwd: &Path, prompt: &str) -> std::io::Result<()> {
    for (marker, hidden) in [("[background ", false), ("[background-hidden ", true)] {
        let Some(delay) = prompt
            .split(marker)
            .nth(1)
            .and_then(|rest| rest.split(']').next())
        else {
            continue;
        };
        // The outer shell leads a new session, as the Agents' tool shells do, and exits as soon
        // as the job is started; a hidden job keeps a readable parent shell that waits for it.
        let script = if hidden {
            "(\"$0\" \"$@\" --hidden; :) &"
        } else {
            "\"$0\" \"$@\" &"
        };
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .arg(std::env::current_exe()?)
            .args([JOB_COMMAND, delay.trim()])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Like Bun, the tool shell keeps none of the plugin's descriptors, the Node's attempt lock
        // included; marking them close-on-exec keeps the spawn's own error pipe working.
        // SAFETY: setsid and close_range are async-signal-safe and touch only the forked child.
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
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.status()?;
    }
    Ok(())
}

/// Runs as the job: optionally hides its environment, records its PID, and writes the leak file
/// in its directory after the delay.
pub(super) fn run(args: &[String]) -> std::io::Result<()> {
    if args.iter().any(|arg| arg == "--hidden") {
        // SAFETY: prctl with scalar arguments only changes this process's dumpable flag.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let delay = args
        .first()
        .and_then(|delay| delay.parse().ok())
        .ok_or_else(|| std::io::Error::other("background job needs a delay"))?;
    std::fs::write("background.pid.tmp", std::process::id().to_string())?;
    std::fs::rename("background.pid.tmp", "background.pid")?;
    std::thread::sleep(Duration::from_millis(delay));
    std::fs::write("background-leak", b"written after the session stopped\n")
}
