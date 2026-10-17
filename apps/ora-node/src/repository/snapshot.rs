//! The two halves of a snapshot clone (Node clone ADR D2): the Node fetches the snapshot into a
//! private transport repository, then the workload user builds the checkout from it with a fixed
//! command sequence, no remote ever being configured.
use super::*;
use crate::git_server::{GitServerAccess, GitServerFailure, TransportRepository};
use crate::managed::{AsNode, CloneHost};
use gitlancer::{GitCommand, GitIntent};
use std::path::{Path, PathBuf};

/// Kind of the transport repositories clones use.
const TRANSPORT_KIND: &str = "clone";
/// Passes of the snapshot fetch a clone may start in all, across restarts (ADR D4).
pub(super) const MAX_FETCH_PASSES: u32 = 3;
/// Commands of the workload sequence; the last one leaves the checkout complete.
pub(super) const WORKLOAD_STEPS: u32 = 3;
/// The workload command that takes the snapshot out of the transport repository.
const TRANSPORT_STEP: u32 = 1;

/// What the Node's snapshot fetch found.
pub(super) enum Fetched {
    /// The transport repository holds the base commit at `refs/ora/input` and belongs to the
    /// workload user now.
    Ready,
    Failed(CloneFailureCode),
    /// The Node is stopping: the pass ends without a verdict and the next start fetches again.
    Stopped,
}

/// Where the clone's transport repository lives.
pub(super) fn transport_path(config: &CloneConfig, record: &CloneExecution) -> PathBuf {
    TransportRepository::location(
        &config.repository_root,
        TRANSPORT_KIND,
        &record.command.execution_id,
    )
}

/// Whether the clone's workload sequence may still read its transport repository: it has not
/// finished the step that takes the snapshot from it. That step is never preceded by another
/// fetch pass, so the repository must outlive a Node restart.
pub(super) fn reads_transport(record: &CloneExecution) -> bool {
    match &record.progress {
        CloneProgress::Pending(phase) | CloneProgress::Unknown(phase) => match phase {
            ClonePhase::Dispatched { step, .. } => *step <= TRANSPORT_STEP,
            ClonePhase::Reserved
            | ClonePhase::DirectoryCreated { .. }
            | ClonePhase::Fetching { .. } => false,
        },
        CloneProgress::Completed(_) => false,
    }
}

/// Fetches the snapshot ref as the Node and checks that it is the base commit, then hands the
/// transport repository, which holds no credential, to the workload user.
pub(super) fn fetch(
    host: &CloneHost,
    access: &GitServerAccess,
    config: &CloneConfig,
    record: &CloneExecution,
) -> Fetched {
    let spec = &record.command.payload.spec;
    let node = AsNode(host);
    let transport = match TransportRepository::create(
        &node,
        &config.repository_root,
        TRANSPORT_KIND,
        &record.command.execution_id,
    ) {
        Ok(transport) => transport,
        Err(error) => {
            ora_logging::ora_warn!(execution_id = %record.command.execution_id.as_str(), error = %error, "clone transport repository could not be created");
            return Fetched::Failed(CloneFailureCode::OperationFailed);
        }
    };
    let fetched = access.fetch(
        &node,
        &transport,
        &spec.repository_url,
        &record.command.execution_id,
        &[format!("{}:refs/ora/input", spec.snapshot_ref.as_str())],
        &|| host.stopping(),
    );
    let failure = match fetched {
        Ok(()) => None,
        // The Git Server answers missing and unreadable refs the same way.
        Err(GitServerFailure::Denied | GitServerFailure::MissingRef) => {
            Some(CloneFailureCode::SnapshotUnavailable)
        }
        Err(GitServerFailure::Unavailable) => Some(CloneFailureCode::SourceUnavailable),
        Err(GitServerFailure::Stopped) => return Fetched::Stopped,
    };
    if let Some(failure) = failure {
        ora_logging::ora_warn!(execution_id = %record.command.execution_id.as_str(), failure = ?failure, "clone snapshot fetch failed");
        return Fetched::Failed(failure);
    }
    match transport.commit_of(&node, "refs/ora/input") {
        Ok(Some(commit)) if commit == spec.base_commit => {}
        Ok(_) => {
            ora_logging::ora_warn!(execution_id = %record.command.execution_id.as_str(), "clone snapshot ref does not point at the base commit");
            return Fetched::Failed(CloneFailureCode::BaseCommitMismatch);
        }
        Err(_) => return Fetched::Failed(CloneFailureCode::OperationFailed),
    }
    if let Err(error) = transport.hand_over(host.workload_uid()) {
        ora_logging::ora_warn!(execution_id = %record.command.execution_id.as_str(), error = %error, "clone transport repository could not be handed over");
        return Fetched::Failed(CloneFailureCode::OperationFailed);
    }
    transport.keep();
    Fetched::Ready
}

/// The workload command `step` of the checkout sequence, with clone's hardened policy:
/// initialize an empty repository on the Workspace branch, take the base commit from the
/// transport repository straight into that branch, and check it out.
pub(super) fn workload_command(
    config: &CloneConfig,
    record: &CloneExecution,
    step: u32,
) -> Result<GitCommand, Error> {
    let branch = record.command.payload.spec.workspace_ref.branch();
    let transport = transport_path(config, record);
    let transport = utf8(&transport)?;
    let args: Vec<String> = match step {
        0 => vec![
            "init".into(),
            "--quiet".into(),
            "--template=".into(),
            format!("--initial-branch={branch}"),
        ],
        // The only command that may read a local repository: the transport repository.
        TRANSPORT_STEP => vec![
            "-c".into(),
            "protocol.file.allow=always".into(),
            "-c".into(),
            "gc.auto=0".into(),
            "-c".into(),
            "maintenance.auto=false".into(),
            "fetch".into(),
            "--quiet".into(),
            "--no-tags".into(),
            "--no-write-fetch-head".into(),
            "--no-recurse-submodules".into(),
            "--update-head-ok".into(),
            "--".into(),
            transport.to_owned(),
            format!("refs/ora/input:refs/heads/{branch}"),
        ],
        _ => vec!["reset".into(), "--quiet".into(), "--hard".into()],
    };
    let mut command = GitCommand::new(
        record.target.path.clone(),
        args,
        config.environment()?,
        GitIntent::Mutating,
    );
    config.constrain(&mut command);
    Ok(command)
}

/// Protocol and Git paths are UTF-8; anything else is a configuration fault.
fn utf8(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::Configuration("non-UTF-8 clone path".into()))
}
