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
//!
//! Two markers start processes from the Agent itself instead:
//!
//! - `[helper]` starts a long-lived helper in the Agent's own session, the way an Agent launches
//!   a language server lazily, and records its PID in `helper.pid`; a later `[helper]` reports
//!   whether that same helper still runs;
//! - `[respawn]` makes the Agent restart a `setsid` job each time it dies, until the Agent exits,
//!   as an Agent that supervises a tool job and keeps it running after it answered; it records
//!   the current job's PID in `respawn.pid`. `[respawn-for <ms>]` instead restarts short-lived
//!   jobs, each living 200 ms, and starts none once `<ms>` milliseconds passed after the answer;
//! - `[late-job <ms>]` starts a helper in the Agent's session that starts a `setsid` sentinel job
//!   at once and, `<ms>` milliseconds after that sentinel died, silently starts another `setsid`
//!   job and records its PID in `late-job.pid`, as a language server that launches a worker of
//!   its own later. Only the turn process fence kills the sentinel, so the second job starts
//!   after the fence, whatever the fence took;
//! - `[watch-job]` starts a `setsid` job the Agent watches, recorded in `watch-job.pid`; once it
//!   dies after a cancel was answered, the Agent reports that in a message.
//!
//! The Agent writes `answered` in the session's directory whenever it answers a cancel.

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

/// Starts or finds the `[helper]` process; the answer the prompt reports about it, if asked.
pub(super) fn helper(cwd: &Path, prompt: &str) -> std::io::Result<Option<String>> {
    if !prompt.contains("[helper]") {
        return Ok(None);
    }
    let record = cwd.join("helper.pid");
    if let Ok(pid) = std::fs::read_to_string(&record)
        && let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
        && stat.contains("(sleep) S")
    {
        return Ok(Some(format!("helper {} running", pid.trim())));
    }
    // No new session or group: the helper stays wherever the Agent runs. The handle is dropped
    // without waiting, as an Agent that keeps a helper for its lifetime does.
    let child = Command::new("/bin/sleep")
        .arg("600")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::fs::write(&record, child.id().to_string())?;
    Ok(Some(format!("helper {} started", child.id())))
}

/// A sleep of `seconds` in a session of its own, as a tool shell that calls `setsid` starts it.
fn session_job(cwd: &Path, seconds: &str) -> Command {
    let mut command = Command::new("/bin/sleep");
    command.arg(seconds);
    in_new_session(cwd, command)
}

/// Makes `command` start in a session of its own in `cwd`, detached from the Agent's stdio.
fn in_new_session(cwd: &Path, mut command: Command) -> Command {
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches only the forked child.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
}

/// The delay a `[<marker> <ms>]` marker of `prompt` names.
fn delay(prompt: &str, marker: &str) -> Option<Duration> {
    let rest = prompt.split(&format!("[{marker} ")).nth(1)?;
    rest.split(']')
        .next()?
        .trim()
        .parse()
        .ok()
        .map(Duration::from_millis)
}

/// Starts the `[respawn]` supervisor on its own thread; it runs until the Agent exits, or until
/// the `[respawn-for <ms>]` delay passed after the answer.
pub(super) fn respawn(cwd: &Path, prompt: &str) {
    let limit = delay(prompt, "respawn-for");
    if !prompt.contains("[respawn]") && limit.is_none() {
        return;
    }
    let cwd = cwd.to_path_buf();
    std::thread::spawn(move || {
        let mut answered_at = None;
        loop {
            if let Some(limit) = limit {
                if answered_at.is_none() && cwd.join("answered").exists() {
                    answered_at = Some(std::time::Instant::now());
                }
                if answered_at.is_some_and(|at| at.elapsed() >= limit) {
                    return;
                }
            }
            // One job at a time: the next one starts as soon as the last one died.
            let seconds = if limit.is_some() { "0.2" } else { "60" };
            match session_job(&cwd, seconds).spawn() {
                Ok(mut job) => {
                    let _ = std::fs::write(cwd.join("respawn.pid"), job.id().to_string());
                    let _ = job.wait();
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    });
}

/// Starts the `[late-job <ms>]` helper in the Agent's own session.
pub(super) fn late_job(cwd: &Path, prompt: &str) -> std::io::Result<()> {
    let Some(delay) = delay(prompt, "late-job") else {
        return Ok(());
    };
    // No new session: the helper stays where the Agent runs, so it is exempt; only the jobs it
    // starts leave for sessions of their own.
    Command::new("/bin/sh")
        .args([
            "-c",
            "/usr/bin/setsid /bin/sleep 600 & wait $!; sleep \"$0\"; \
             /usr/bin/setsid /bin/sleep 60 & echo $! > late-job.pid.tmp; \
             mv late-job.pid.tmp late-job.pid; wait",
        ])
        .arg(format!("{:.3}", delay.as_secs_f64()))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// Starts the `[watch-job]` job and records its PID.
pub(super) fn watched_job(cwd: &Path) -> std::io::Result<std::process::Child> {
    let job = session_job(cwd, "60").spawn()?;
    std::fs::write(cwd.join("watch-job.pid"), job.id().to_string())?;
    Ok(job)
}
