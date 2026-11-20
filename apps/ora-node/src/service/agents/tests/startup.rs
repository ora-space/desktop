//! Startup checks that run when the Agent configuration is validated.
use super::ora_git::Logs;
use pretty_assertions::assert_eq;

/// The warning the mark-scan prerequisite logs.
const WARNING: &str = "session stop cannot read workload environments";

/// Validates an Agent configuration whose workload runs as `workload_uid`, as service startup
/// does, and returns how many prerequisite warnings it logged.
fn warnings(workload_uid: Option<u32>) -> Result<usize, Box<dyn std::error::Error>> {
    let logs = Logs::default();
    let writer = logs.clone();
    ora_logging::with_recorded_trace_logging(
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone()),
        || -> Result<(), Box<dyn std::error::Error>> {
            let root = tempfile::tempdir()?;
            let workload =
                workload_uid.map(|_| super::directory(&root.path().join("agents"), 0o711));
            let config = super::config(root.path(), workload_uid, workload);
            super::super::validate(config.agent.as_ref().ok_or("agent")?, &config)?;
            Ok(())
        },
    )?;
    let text = String::from_utf8(
        logs.0
            .lock()
            .map_err(|_| std::io::Error::other("log buffer poisoned"))?
            .clone(),
    )?;
    Ok(text.lines().filter(|line| line.contains(WARNING)).count())
}

/// Validating the Agent configuration checks the mark-scan prerequisite: a workload of the Node's
/// own user passes silently, and one of another user is warned about unless the Node can read it
/// with `CAP_SYS_PTRACE`. The configuration is accepted either way.
#[test]
fn agent_startup_warns_when_the_mark_scan_cannot_read_the_workload()
-> Result<(), Box<dyn std::error::Error>> {
    ora_logging::initialize_test_clock();
    // SAFETY: geteuid only reads this process's identity.
    let uid = unsafe { libc::geteuid() };
    let ptrace = ora_utils::process::linux_has_effective_capability(/*capability*/ 19)?;
    assert_eq!(
        (
            warnings(None)?,
            warnings(Some(uid))?,
            warnings(Some(uid ^ 1))?
        ),
        (0, 0, usize::from(!ptrace))
    );
    Ok(())
}
