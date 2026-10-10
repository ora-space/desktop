//! Resuming a session from the prior Revision of its Issue (Node restore ADR D1, D4).
//!
//! The restore runs after the checkout is resolved and before the checkout is handed to the
//! workload user or any plugin starts, so a failure ends the session without an agent ever having
//! existed. When the remote history no longer contains a base of the restored commits, the Agent
//! is told so in its first turn, which also puts the note into the session history and Thread.

use super::end::SessionEnd;
use super::ports::{PriorRevisionRestore, RestoreRequest, Restored};
use ora_logging::{ora_info, ora_warn};
use ora_node_protocol::{
    AgentSessionSpec, CommitId, ContentBlock, EndSessionReason, ExecutionId, OperationId,
};
use std::path::Path;
use tokio::sync::watch;

/// Restores the prior Revision the session names, if any, and appends the divergence note to the
/// initial turn when the remote history was rewritten. A Node stopping meanwhile cancels the
/// session like a stopping conversation; the restore's Git refuses new commands once the Node
/// stops, so an abandoned restore ends on its own.
pub(super) async fn restore_prior<R: PriorRevisionRestore>(
    restorer: &R,
    operation: &OperationId,
    execution: &ExecutionId,
    checkout: &Path,
    spec: &mut AgentSessionSpec,
    mut stopping: watch::Receiver<bool>,
) -> Result<(), SessionEnd> {
    let Some(prior) = spec.prior_revision.clone() else {
        return Ok(());
    };
    let request = RestoreRequest {
        operation: operation.clone(),
        execution: execution.clone(),
        checkout: checkout.to_path_buf(),
        prior,
    };
    let restored = tokio::select! {
        biased;
        _ = stopping.wait_for(|stop| *stop) => {
            return Err(SessionEnd::requested(EndSessionReason::Cancelled));
        }
        restored = restorer.restore(request) => restored,
    };
    match restored {
        Ok(Restored::OnRemoteHistory) => {
            ora_info!(execution_id = %execution, "prior Revision restored");
            Ok(())
        }
        Ok(Restored::Diverged {
            final_commit,
            base_commit,
            branch,
        }) => {
            ora_info!(execution_id = %execution, "prior Revision restored onto rewritten remote history");
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

/// The fixed-format note appended to the first turn when `origin/<branch>` lost a base commit.
fn divergence_note(final_commit: &CommitId, base_commit: &CommitId, branch: &str) -> String {
    let (final_commit, base_commit) = (final_commit.as_str(), base_commit.as_str());
    format!(
        "Note from Ora: this run resumed the previous run's work at commit {final_commit}. \
         That work was built on commit {base_commit}, which origin/{branch} no longer contains: \
         the remote history was rewritten since. The branch {branch} points at the resumed work \
         and origin/{branch} is unchanged; reconcile them before relying on either."
    )
}
