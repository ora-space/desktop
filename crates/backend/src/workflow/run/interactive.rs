//! Coordination for human interaction with workflow node sessions.

#[cfg(test)]
mod cleanup_tests;
mod completion;
#[cfg(test)]
mod completion_tests;
#[cfg(test)]
mod inactivity_tests;
mod session;
mod turns;

use ora_domain::WorkflowNodeRunId;
use std::collections::HashSet;

pub(crate) use completion::{
    CompletionApproval, claim_node_for_completion, completion_file_changes, prepare_completion,
    revalidate_completion,
};
pub(crate) use session::{begin_human_turn, end_human_turn};
pub(crate) use turns::WorkflowSessionTurns;

/// Tracks node runs currently claimed by an in-process manual completion.
///
/// This transient gate prevents a follow-up prompt from racing a completion without introducing a
/// persisted state that could outlive the backend process.
pub(crate) type CompletingNodeRuns = std::sync::Mutex<HashSet<WorkflowNodeRunId>>;
