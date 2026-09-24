//! An automatic-retry chain as one rollback unit.
//!
//! Automatic retries never roll files back, so each attempt of a chain builds on the worktree
//! the previous attempt left. When the chain is exhausted and the run is resumed by hand, the
//! chain — every attempt of the node in its round since the last start, restart, or resume,
//! listed on the live row as `payload.retry_chain` — is rolled back as one failure: its baseline
//! is the first attempt (start time and pre-node checkpoint), and its file changes are those of
//! every attempt. A row with no chain keeps exactly its own facts.

use super::{ParsedPayload, parse_node_payload};
use ora_application::{FileChange, retry_chain_from_payload};
use ora_domain::WorkflowNodeRun;
use std::collections::HashMap;

/// Rollback facts of one failed or cancelled live row, widened to its whole retry chain.
pub(super) struct ChainBaseline {
    /// Start of the chain's first attempt.
    pub started_at: Option<i64>,
    /// Pre-node checkpoint of the chain's first attempt.
    pub checkpoint: Option<String>,
    /// First checkpoint error recorded anywhere in the chain.
    pub checkpoint_error: Option<String>,
    /// Paths changed by any attempt, first-seen order, with line counts summed over attempts.
    pub file_changes: Vec<FileChange>,
    /// Whether `file_changes` is known to cover the chain: the first attempt has a checkpoint
    /// and every attempt that ran recorded one (an attempt that never started changed nothing).
    pub file_changes_complete: bool,
}

/// Node-run ids of the earlier attempts every live row's chain refers to.
pub(super) fn chain_ids<'a>(live: impl Iterator<Item = &'a WorkflowNodeRun>) -> Vec<String> {
    live.flat_map(|row| retry_chain_from_payload(row.payload.as_deref()))
        .collect()
}

/// Widens `live` to its chain, reading earlier attempts from `earlier` (soft-deleted rows by
/// id). A chain whose earlier attempt cannot be read has no usable baseline.
pub(super) fn chain_baseline(
    live: &WorkflowNodeRun,
    earlier: &HashMap<String, WorkflowNodeRun>,
) -> ChainBaseline {
    let own = parse_node_payload(live.payload.as_deref());
    let chain = retry_chain_from_payload(live.payload.as_deref());
    if chain.is_empty() {
        return ChainBaseline {
            started_at: live.started_at,
            file_changes_complete: own.checkpoint.is_some(),
            checkpoint: own.checkpoint,
            checkpoint_error: own.checkpoint_error,
            file_changes: own.file_changes,
        };
    }
    let Some(attempts) = chain
        .iter()
        .map(|id| earlier.get(id))
        .collect::<Option<Vec<_>>>()
    else {
        return ChainBaseline {
            started_at: live.started_at,
            checkpoint: None,
            checkpoint_error: own.checkpoint_error,
            file_changes: own.file_changes,
            file_changes_complete: false,
        };
    };
    let first = attempts[0];
    let parsed: Vec<(Option<i64>, ParsedPayload)> = attempts
        .iter()
        .map(|row| (row.started_at, parse_node_payload(row.payload.as_deref())))
        .chain(std::iter::once((live.started_at, own)))
        .collect();
    let checkpoint = parsed[0].1.checkpoint.clone();
    let file_changes_complete = checkpoint.is_some()
        && parsed
            .iter()
            .all(|(started_at, payload)| started_at.is_none() || payload.checkpoint.is_some());
    let checkpoint_error = parsed
        .iter()
        .find_map(|(_, payload)| payload.checkpoint_error.clone());
    let mut file_changes: Vec<FileChange> = Vec::new();
    for change in parsed
        .into_iter()
        .flat_map(|(_, payload)| payload.file_changes)
    {
        match file_changes
            .iter_mut()
            .find(|known| known.path == change.path)
        {
            Some(known) => {
                known.additions = known.additions.saturating_add(change.additions);
                known.deletions = known.deletions.saturating_add(change.deletions);
            }
            None => file_changes.push(change),
        }
    }
    ChainBaseline {
        started_at: first.started_at,
        checkpoint,
        checkpoint_error,
        file_changes,
        file_changes_complete,
    }
}
