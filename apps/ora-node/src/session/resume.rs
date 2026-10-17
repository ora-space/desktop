//! Resuming a session from the prior Revision of its Issue (Node restore ADR D1, D4).
//!
//! The restore runs after the checkout is resolved and before the checkout is handed to the
//! workload user or any plugin starts, so a failure ends the session without an agent ever having
//! existed. When this Workspace's snapshot no longer contains the base of the restored commits,
//! the Agent is told so in its first turn, which also puts the note into the session history and
//! Thread.

use super::end::SessionEnd;
use super::ports::{PriorRevisionRestore, RestoreCancel, RestoreFailure, RestoreRequest, Restored};
use ora_logging::{ora_info, ora_warn};
use ora_node_db::ClonedCheckout;
use ora_node_protocol::{
    AgentSessionSpec, CommitId, ContentBlock, EndSessionReason, ExecutionId, OperationId,
};
use std::future::Future;
use tokio::sync::watch;

/// Restores the prior Revision the session names, if any, and appends the divergence note to the
/// initial turn when the remote history was rewritten.
///
/// A Node stopping meanwhile cancels the session like a stopping conversation, and an
/// `EndSession` ends it with its own reason (`ended` resolves to that end). Either way the restore
/// is told to stop and awaited first: the session must not end, and delivery must not read the
/// checkout, while restore Git may still write it.
pub(super) async fn restore_prior<R: PriorRevisionRestore>(
    restorer: &R,
    operation: &OperationId,
    execution: &ExecutionId,
    checkout: &ClonedCheckout,
    spec: &mut AgentSessionSpec,
    mut stopping: watch::Receiver<bool>,
    ended: impl Future<Output = SessionEnd>,
) -> Result<(), SessionEnd> {
    let Some(prior) = spec.prior_revision.clone() else {
        return Ok(());
    };
    let request = RestoreRequest {
        operation: operation.clone(),
        execution: execution.clone(),
        checkout: checkout.path.clone(),
        branch: checkout.spec.workspace_ref.branch().to_owned(),
        base_commit: checkout.spec.base_commit.clone(),
        prior,
    };
    let cancel = RestoreCancel::default();
    let restore = restorer.restore(request, cancel.clone());
    tokio::pin!(restore);
    let end = tokio::select! {
        biased;
        _ = stopping.wait_for(|stop| *stop) => SessionEnd::requested(EndSessionReason::Cancelled),
        end = ended => end,
        restored = &mut restore => return note_restored(execution, spec, restored),
    };
    cancel.cancel();
    let _ = restore.await;
    Err(end)
}

/// Turns the restore's answer into the session's next step: the agent starts, with a note in its
/// first turn when the snapshot lost the prior base, or the session ends without an agent.
fn note_restored(
    execution: &ExecutionId,
    spec: &mut AgentSessionSpec,
    restored: Result<Restored, RestoreFailure>,
) -> Result<(), SessionEnd> {
    match restored {
        Ok(Restored::OnSnapshotHistory) => {
            ora_info!(execution_id = %execution, "prior Revision restored");
            Ok(())
        }
        Ok(Restored::Diverged {
            final_commit,
            base_commit,
            branch,
        }) => {
            ora_info!(execution_id = %execution, "prior Revision restored onto a rewritten snapshot history");
            spec.initial_turn.content.push(ContentBlock::Text {
                text: divergence_note(&final_commit, &base_commit, &branch),
            });
            Ok(())
        }
        Err(failure) => {
            ora_warn!(execution_id = %execution, failure = %failure, "prior Revision could not be restored; the agent does not start");
            Err(SessionEnd::agent_failed(failure.detail()))
        }
    }
}

/// The fixed-format note appended to the first turn when the snapshot lost the prior base.
fn divergence_note(final_commit: &CommitId, base_commit: &CommitId, branch: &str) -> String {
    let (final_commit, base_commit) = (final_commit.as_str(), base_commit.as_str());
    format!(
        "Note from Ora: this run resumed the previous run's work at commit {final_commit}. \
         That work was built on commit {base_commit}, which is not in the history of this \
         Workspace's snapshot: the external repository's history was rewritten since. The local \
         branch {branch} points at the resumed work; run `ora-git fetch upstream` to see the \
         external repository's latest content and reconcile it."
    )
}
