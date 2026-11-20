//! The attempt mark ties every process of one attempt to it, including the tool jobs that left the
//! attempt's process groups (`setsid` plus `&`), which no clue reaches.
//!
//! Every process the attempt's spawn gate starts carries `ORA_SESSION_ATTEMPT=<attempt>`, and its
//! descendants inherit it. A durable scope beside the lock records when the attempt began and as
//! which user its processes run, so a restarted Node scans for, and kills, exactly what the
//! attempt may own. Stop evidence needs a clean scan; terminating freezes and kills what a scan
//! claims. What the scan cannot see, and why a forged mark cannot widen it, is documented on
//! [`ora_utils::process::MarkScope`].

use crate::managed::attempt_lock::StopEvidence;
use ora_node_db::AttemptId;
use ora_process::ProcessIdentity;
use ora_utils::process::{EnvironmentMark, MarkScope, linux_boot_ticks};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// The environment variable every process of a session attempt inherits.
const NAME: &str = "ORA_SESSION_ATTEMPT";

/// The window and user one attempt's processes can have.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    since_ticks: u64,
    uid: u32,
}

/// The mark of one attempt. An attempt identity is a UUID, so it always forms a valid entry.
pub(super) fn mark(attempt: &AttemptId) -> io::Result<EnvironmentMark> {
    EnvironmentMark::new(NAME, attempt.as_str())
}

/// Durably records the attempt's scope before the ledger takes responsibility for it. The window
/// opens here, before the first spawn, so every process of the attempt starts inside it.
pub(super) fn record(
    directory: &Path,
    attempt: &AttemptId,
    identity: ProcessIdentity,
) -> io::Result<()> {
    let uid = match identity {
        // SAFETY: geteuid only reads this process's credentials.
        ProcessIdentity::Inherit => unsafe { libc::geteuid() },
        ProcessIdentity::Linux(identity) => identity.uid(),
    };
    let scope = Scope {
        since_ticks: linux_boot_ticks()?,
        uid,
    };
    let mut file = fs::File::create(path(directory, attempt))?;
    file.write_all(&serde_json::to_vec(&scope)?)?;
    file.sync_all()?;
    fs::File::open(directory)?.sync_all()
}

/// Reads the scope back. Attempts recorded before marks existed have none, and none of their
/// processes carries a mark, so their evidence stays the lock and the clues alone.
pub(super) fn scope(directory: &Path, attempt: &AttemptId) -> io::Result<Option<MarkScope>> {
    let mut bytes = Vec::new();
    match fs::File::open(path(directory, attempt)) {
        Ok(file) => file.take(1024).read_to_end(&mut bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let scope: Scope = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::other("invalid attempt mark scope"))?;
    Ok(Some(MarkScope::new(
        mark(attempt)?,
        scope.since_ticks,
        scope.uid,
    )))
}

/// Stopped only after two clean scans in a row: a process that forks and exits while one scan
/// passes over its new child's slot, after PID wrap-around, cannot hide its child from both. An
/// unreadable procfs or an undecided process is never evidence that nothing runs.
pub(super) fn evidence(scope: &MarkScope) -> StopEvidence {
    for _ in 0..2 {
        match scope.scan() {
            Ok(scan) if !scan.claimed.is_empty() => return StopEvidence::Running,
            Ok(scan) if scan.undecided == 0 => {}
            Ok(_) | Err(_) => return StopEvidence::Unknown,
        }
    }
    StopEvidence::Stopped
}

/// Whether this Node can read the environments its attempts' processes run with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MarkReach {
    /// Node and workload share a user, so every environment of the attempt is readable.
    SameUser,
    /// The workload runs as another user, but `CAP_SYS_PTRACE` lets the Node read it.
    Ptrace,
    /// The workload runs as another user the Node cannot read: every live process of that user
    /// in an attempt's window leaves its stop evidence Unknown.
    Blind { node: u32, workload: u32 },
}

/// Decides the deployment prerequisite of the mark scan from the Node's identity, the
/// configured workload user, and whether the Node holds `CAP_SYS_PTRACE`.
pub(crate) fn reach(node: u32, workload: Option<u32>, ptrace: bool) -> MarkReach {
    match workload {
        None => MarkReach::SameUser,
        Some(workload) if workload == node => MarkReach::SameUser,
        Some(_) if ptrace => MarkReach::Ptrace,
        Some(workload) => MarkReach::Blind { node, workload },
    }
}

/// Warns, without refusing to start, when the mark scan cannot read the workload: stops then
/// end as `stop_unconfirmed` more often, never with false evidence.
pub(crate) fn warn_unless_readable(workload: Option<u32>) {
    // SAFETY: geteuid only reads this process's credentials.
    let node = unsafe { libc::geteuid() };
    // An unreadable capability set counts as missing: the warning then errs toward caution.
    let ptrace =
        ora_utils::process::linux_has_effective_capability(CAP_SYS_PTRACE).unwrap_or(false);
    if let MarkReach::Blind { node, workload } = reach(node, workload, ptrace) {
        ora_logging::ora_warn!(
            node_uid = node,
            workload_uid = workload,
            "session stop cannot read workload environments: run the Node as the workload user or grant it CAP_SYS_PTRACE; until then a stop with any live workload process stays unconfirmed"
        );
    }
}

/// The Linux capability that lets a process read other users' `/proc/<pid>/environ`.
const CAP_SYS_PTRACE: u32 = 19;

/// Keeps the scope beside the attempt's lock and clues, under the same retired prefix.
fn path(directory: &Path, attempt: &AttemptId) -> PathBuf {
    directory.join(format!("{attempt}.mark"))
}

#[cfg(test)]
mod tests {
    use super::{MarkReach, reach};
    use pretty_assertions::assert_eq;

    /// The deployment prerequisite holds for a shared user or `CAP_SYS_PTRACE`; anything else is
    /// reported, with both users named, so the Node can warn.
    #[test]
    fn the_mark_scan_needs_a_shared_user_or_ptrace() {
        assert_eq!(
            [
                reach(
                    /*node*/ 1000, /*workload*/ None, /*ptrace*/ false
                ),
                reach(/*node*/ 1000, Some(1000), /*ptrace*/ false),
                reach(/*node*/ 0, Some(1000), /*ptrace*/ true),
                reach(/*node*/ 0, Some(1000), /*ptrace*/ false),
            ],
            [
                MarkReach::SameUser,
                MarkReach::SameUser,
                MarkReach::Ptrace,
                MarkReach::Blind {
                    node: 0,
                    workload: 1000
                },
            ]
        );
    }
}
