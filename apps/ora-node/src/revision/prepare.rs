//! Preparation runs once per delivery: snapshot, and freeze the final commit, the declaration and
//! the history's bytes, size and digest under the Node home before anything is pushed or uploaded
//! (Node delivery push ADR D3).
use super::HISTORY_FILE;
use super::snapshot::{DeliveryGit, Snapshot, SnapshotFailure, SnapshotRequest};
use gitlancer::GitRunner;
use ora_node_db::{DeliveryPlan, FrozenOutcome};
use ora_node_protocol::*;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Everything the worker resolved from the ledger for one delivery's preparation.
#[derive(Clone, Debug)]
pub(crate) struct Preparation {
    pub(crate) execution: ExecutionId,
    pub(crate) spec: DeliverRevisionSpec,
    pub(crate) checkout: PathBuf,
    /// The sealed session JSONL; nothing writes to it anymore.
    pub(crate) history: PathBuf,
    pub(crate) author: GitIdentity,
    /// The incarnation that prepares the bytes is the one the declaration names, even when a
    /// later incarnation finishes the upload.
    pub(crate) node: NodeRuntimeIdentity,
    /// The workload user delivery Git runs as, when the deployment separates it from the Node.
    pub(crate) owner: Option<u32>,
}

/// The per-execution directory name: execution IDs are opaque protocol text, so the name is
/// derived from a digest instead of the ID itself to stay a single safe path component.
pub(crate) fn directory_name(execution: &ExecutionId) -> String {
    ora_utils::hash::sha256_hex(execution.as_str().as_bytes())
}

/// Produces the frozen plan, or the definitive failure code. On failure nothing stays frozen.
pub(crate) fn prepare<R: GitRunner>(
    git: &DeliveryGit<R>,
    root: &Path,
    job: &Preparation,
) -> Result<DeliveryPlan, RevisionFailureCode> {
    let directory = directory_name(&job.execution);
    let frozen = root.join(&directory);
    let result = freeze(git, &frozen, &directory, job);
    if result.is_err() {
        let _ = fs::remove_dir_all(&frozen);
    }
    result
}

/// Writes the history into a fresh directory and describes the delivery as it will be reported.
fn freeze<R: GitRunner>(
    git: &DeliveryGit<R>,
    frozen: &Path,
    directory: &str,
    job: &Preparation,
) -> Result<DeliveryPlan, RevisionFailureCode> {
    // Anything here predates a durable plan, so it is an abandoned attempt and never reused.
    match fs::remove_dir_all(frozen) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(local(error, RevisionFailureCode::SnapshotFailed)),
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(/*recursive*/ true).mode(/*mode*/ 0o700);
    builder
        .create(frozen)
        .map_err(|error| local(error, RevisionFailureCode::SnapshotFailed))?;
    let history_path = frozen.join(HISTORY_FILE);
    copy_synced(&job.history, &history_path)
        .map_err(|error| local(error, RevisionFailureCode::HistoryUnavailable))?;
    // Agents now run as the workload user, but a checkout a session of an older Node worked in
    // (when the agent ran as the Node user) can still hold files the Node user owns with
    // owner-only modes, which delivery Git — running as the workload user like clone — could not
    // read. The session has ended, so nothing writes the checkout now; hand it back to the
    // workload user without following links, so a link the agent left cannot redirect this
    // privileged change outside the checkout. A checkout that is already right costs only a walk.
    if let Some(owner) = job.owner {
        ora_utils::fs::own_tree_no_follow(&job.checkout, owner, owner)
            .map_err(|error| local(error, RevisionFailureCode::SnapshotFailed))?;
    }
    let snapshot = git
        .snapshot(&SnapshotRequest {
            checkout: &job.checkout,
            revision_ref: &job.spec.revision_ref,
            base_commit: &job.spec.base_commit,
            prior_final_commit: job
                .spec
                .prior_revision
                .as_ref()
                .map(|prior| &prior.final_commit),
            author: &job.author,
            scratch: directory,
        })
        .map_err(|failure| match failure {
            SnapshotFailure::CheckoutUnavailable => RevisionFailureCode::CheckoutUnavailable,
            SnapshotFailure::SnapshotFailed => RevisionFailureCode::SnapshotFailed,
        })?;
    let history = stored(&job.spec.history_key, &history_path)
        .map_err(|error| local(error, RevisionFailureCode::HistoryUnavailable))?;
    let outcome = match snapshot {
        Snapshot::Unchanged { final_commit } => FrozenOutcome::Unchanged(RevisionUnchanged {
            node: job.node.clone(),
            final_commit,
            base_commit: job.spec.base_commit.clone(),
            revision_ref: job.spec.revision_ref.clone(),
            history,
        }),
        Snapshot::Changed { final_commit } => FrozenOutcome::Delivered(RevisionDelivered {
            node: job.node.clone(),
            final_commit,
            base_commit: job.spec.base_commit.clone(),
            revision_ref: job.spec.revision_ref.clone(),
            history,
        }),
    };
    fs::File::open(frozen)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| local(error, RevisionFailureCode::SnapshotFailed))?;
    Ok(DeliveryPlan {
        directory: directory.to_owned(),
        outcome,
    })
}

/// Measures a frozen object exactly as it will be uploaded.
pub(crate) fn stored(key: &ObjectKey, path: &Path) -> io::Result<StoredObject> {
    Ok(StoredObject {
        key: key.clone(),
        size: fs::metadata(path)?.len(),
        sha256: Sha256Digest::new(ora_utils::hash::sha256_file(path)?),
    })
}

/// Copies a regular file without following a final symlink, and makes the copy durable.
///
/// The source sits in a directory another program wrote (the session root); refusing a symlink
/// keeps the Node from freezing an unrelated file.
fn copy_synced(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = fs::OpenOptions::new()
        .read(/*read*/ true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)?;
    if !input.metadata()?.is_file() {
        return Err(io::Error::other("delivery source is not a regular file"));
    }
    let mut output = fs::OpenOptions::new()
        .write(/*write*/ true)
        .create_new(/*create_new*/ true)
        .mode(/*mode*/ 0o600)
        .open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()
}

/// Keeps the local cause in the Node log; the protocol only carries the code.
fn local(error: io::Error, failure: RevisionFailureCode) -> RevisionFailureCode {
    ora_logging::ora_warn!(failure = ?failure, error = %error, "Revision delivery could not freeze its objects");
    failure
}
