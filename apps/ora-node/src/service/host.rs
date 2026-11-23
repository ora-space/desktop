//! Kernel settings a deployment must provide before the Node opens anything.

use crate::ProcessConfig;
use std::io;
use std::path::Path;

/// Refuses a separate workload user unless the kernel stops it from hard-linking files it does
/// not own (`fs.protected_hardlinks = 1`, read from `sysctl`).
///
/// The Node, as root, hands checkouts to the workload user. Its walk leaves a file alone while
/// another name links it, but that only stops links that stay in place: an Agent that links a
/// root-owned file (an installed agent package, Node state) into a checkout and removes the link
/// right after the walk pinned it gets the file re-owned. Only this setting keeps the workload user
/// from linking files it neither owns nor may read and write, so it is the guarantee. It is
/// checked here so a misconfigured host never starts, and again before every walk
/// (see [`crate::hand_over`]) because it is global host state that can change later.
pub(super) fn require_protected_hardlinks(
    process: &ProcessConfig,
    sysctl: &Path,
) -> io::Result<()> {
    if process.workload_uid.is_none() {
        return Ok(());
    }
    crate::hand_over::require_protected_hardlinks(sysctl)
}
