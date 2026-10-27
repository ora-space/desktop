//! Orchestrates human completion of an interactive workflow node.
//!
//! The core (validating, assembling the conversation, diffing the worktree) lives here so the
//! future agent/CLI path can reuse the same commit through the engine. The session stop and the
//! engine commit are done by the caller around [`prepare_completion`].

use super::super::completion_snapshot::{
    CompletionBlocker, CompletionBlockerKind, CompletionSnapshot, TurnOrigin,
    completion_final_text, completion_payload, derive_completion_snapshot, forced_completion_audit,
    parse_completion_snapshot, trailing_assistant_text, unknown_completion_snapshot,
};
use super::super::executor::apply_output_contract;
use super::super::worktree::{capture_worktree_snapshot, compute_file_changes};
use super::CompletingNodeRuns;
use crate::error::{BackendError, ErrorClassification};
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    ApplicationError, Clock, FileChange, WorkflowGraph, WorkflowRunEngineRepository,
};
use ora_contracts::{
    PublicError, WorkflowNodeCompletionBlocker, WorkflowNodeCompletionBlockerKind,
    WorkflowNodeIncompleteParams, WorkflowNodeOutputChangedParams, WorkflowNodeTurnChangedParams,
    WorkflowNodeTurnKey,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{
    SessionId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus, WorkspaceId,
};
use ora_history::read_session_history;
use ora_logging::ora_warn;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// The assembled result of validating and preparing one interactive node completion.
#[derive(Debug)]
pub(crate) struct PreparedCompletion {
    pub node_run_id: WorkflowNodeRunId,
    pub output: Option<String>,
    pub structured_output: Option<serde_json::Value>,
    pub stop_reason: Option<String>,
    pub session_id: Option<SessionId>,
    /// The run's workspace, resolved by the caller because only it owns the runtime directory port.
    pub workspace_id: WorkspaceId,
    /// The payload keys a forced completion must commit with the completion itself.
    pub audit: Option<serde_json::Value>,
}

/// Atomically claims an awaiting interactive node for manual completion under the per-run gate.
///
/// The gate is held while the run and node are validated and the node is inserted into the
/// completing set, so any prompt that arrives after this point is rejected until the completion
/// commits (or fails and releases the claim). Returns the claimed node-run id.
pub(crate) fn claim_node_for_completion(
    pool: &RepositoryPool,
    run_locks: &Arc<KeyedResourceLocks>,
    completing_node_runs: &Arc<CompletingNodeRuns>,
    run_id: &WorkflowRunId,
    node_id: &str,
) -> Result<WorkflowNodeRunId, BackendError> {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let _gate = run_locks.acquire_exclusive(run_id.as_ref());
    let context = repository
        .find_execution_context(run_id)
        .map_err(repository_error)?
        .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
            run_id: run_id.to_string(),
        })?;
    if context.run.status != WorkflowRunStatus::Running {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    let node_run = repository
        .list_node_runs(run_id)
        .map_err(repository_error)?
        .into_iter()
        .find(|node_run| node_run.node_id == node_id)
        .ok_or_else(|| ApplicationError::WorkflowNodeNotFound {
            node_id: node_id.to_string(),
        })?;
    if node_run.status != WorkflowNodeStatus::Pending {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    // `insert` returns false when the node is already being completed, so a second concurrent
    // complete cannot both claim the same awaiting node and prepare against it.
    let inserted = completing_node_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(node_run.id.clone());
    if !inserted {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    Ok(node_run.id)
}

/// Re-validates under the per-run gate that the run is still `Running` and the node still
/// `Pending` before the completion commits, so a completion that raced a cancel (or a concurrent
/// complete) aborts instead of reporting false success.
pub(crate) fn revalidate_completion(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    node_run_id: &WorkflowNodeRunId,
) -> Result<(), BackendError> {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    // Read the node first so every rejection path can name the graph node id, not the node-run id,
    // in the error — consistent with the prompt-policy rejections.
    let node_run = repository
        .find_node_run_by_id(node_run_id)
        .map_err(repository_error)?
        .ok_or_else(|| ApplicationError::WorkflowNodeNotFound {
            node_id: node_run_id.to_string(),
        })?;
    let context = repository
        .find_execution_context(run_id)
        .map_err(repository_error)?
        .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
            run_id: run_id.to_string(),
        })?;
    if context.run.status != WorkflowRunStatus::Running {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_run.node_id,
        }
        .into());
    }
    if node_run.status != WorkflowNodeStatus::Pending {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_run.node_id,
        }
        .into());
    }
    Ok(())
}

/// Validates an interactive node and assembles its completion output from persisted state.
///
/// The run must be `Running`, the node must be awaiting input (`Pending`) and interactive. The
/// final assistant output and the turn identity are read from the session's durable history, and
/// the node's completion snapshot is the authority for the blockers gate: a node that parked
/// before snapshots existed is materialized once from the same derivation, so it passes through
/// exactly the same gate as a new node instead of bypassing it. Intended to run in a blocking
/// closure so the async runtime is not held.
pub(crate) fn prepare_completion(
    pool: &RepositoryPool,
    sessions_root: &Path,
    run_id: &WorkflowRunId,
    node_id: &str,
    force: bool,
) -> Result<PreparedCompletion, BackendError> {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let context = repository
        .find_execution_context(run_id)
        .map_err(repository_error)?
        .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
            run_id: run_id.to_string(),
        })?;
    if context.run.status != WorkflowRunStatus::Running {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    let node_run = repository
        .list_node_runs(run_id)
        .map_err(repository_error)?
        .into_iter()
        .find(|node_run| node_run.node_id == node_id)
        .ok_or_else(|| ApplicationError::WorkflowNodeNotFound {
            node_id: node_id.to_string(),
        })?;
    if node_run.status != WorkflowNodeStatus::Pending {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    // Defensive: a `Pending` node run can only be produced by an interactive node, but the
    // completion contract also requires the frozen graph to declare it interactive.
    let graph = WorkflowGraph::parse(&context.graph_json)
        .map_err(ApplicationError::WorkflowRunGraphParse)?;
    let agent_config = graph
        .execution_node(&node_run.node_id)
        .and_then(|node| node.agent_config.as_ref());
    let interactive = agent_config.is_some_and(|config| config.interactive);
    if !interactive {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    let output_contract = agent_config.and_then(|config| config.output_contract.as_ref());
    let stored = parse_completion_snapshot(node_run.payload.as_deref());
    let (history, derived) = match node_run.session_id.as_ref() {
        Some(session_id) => {
            let history = read_session_history(sessions_root, session_id.as_ref())
                .map_err(|error| BackendError::internal("failed to read session history", error))?;
            // A human reply or a legacy materialization: the stored snapshot keeps any
            // `turn_unconfirmed` marker the opening park wrote, so repair never re-opens or
            // retroactively blocks a node.
            let derived =
                derive_completion_snapshot(&history, output_contract, false, TurnOrigin::Human);
            (Some(history), derived)
        }
        None => (None, unknown_completion_snapshot(false)),
    };
    // The conflicts are decided against the freshly derived turn, before any repair write: a stale
    // completion must be told what changed even when the stored snapshot itself is still coherent.
    if let Some(expected) = stored.as_ref().and_then(|stored| stored.turn_key)
        && derived.turn_key != Some(expected)
    {
        return Err(completion_rejected(PublicError::WorkflowNodeTurnChanged(
            WorkflowNodeTurnChangedParams {
                node_id: node_run.node_id.clone(),
                expected: Some(to_contract_turn_key(expected)),
                actual: derived.turn_key.map(to_contract_turn_key),
            },
        )));
    }
    if let Some(expected) = stored
        .as_ref()
        .and_then(|stored| stored.output_digest.as_deref())
        && Some(expected) != derived.output_digest.as_deref()
    {
        return Err(completion_rejected(PublicError::WorkflowNodeOutputChanged(
            WorkflowNodeOutputChangedParams {
                node_id: node_run.node_id.clone(),
                expected: Some(expected.to_string()),
                actual: derived.output_digest,
            },
        )));
    }
    let snapshot = settle_snapshot(&repository, &node_run.id, stored.as_ref(), derived);
    if !force && !snapshot.blockers.is_empty() {
        return Err(completion_rejected(PublicError::WorkflowNodeIncomplete(
            WorkflowNodeIncompleteParams {
                node_id: node_run.node_id.clone(),
                blockers: snapshot.blockers.iter().map(to_contract_blocker).collect(),
            },
        )));
    }
    let stop_reason = snapshot.stop_reason.clone();
    // A forced completion without a settled turn takes the last assistant reply as its output and
    // never claims a structured output; a settled turn always uses its own contract window.
    let raw_output = match (&history, snapshot.turn_key) {
        (Some(history), Some(_)) => completion_final_text(history),
        (Some(history), None) if force => trailing_assistant_text(history),
        _ => None,
    };
    let (output, structured_output) =
        match apply_output_contract(output_contract, raw_output.clone(), &node_run.node_id) {
            Ok(output) => output,
            Err(_) if force => (raw_output, None),
            Err(_) => {
                return Err(completion_rejected(PublicError::WorkflowNodeIncomplete(
                    WorkflowNodeIncompleteParams {
                        node_id: node_run.node_id.clone(),
                        blockers: snapshot.blockers.iter().map(to_contract_blocker).collect(),
                    },
                )));
            }
        };

    Ok(PreparedCompletion {
        node_run_id: node_run.id,
        output,
        structured_output,
        stop_reason,
        session_id: node_run.session_id,
        workspace_id: context.workspace.id,
        audit: force.then(|| forced_completion_audit(&snapshot)),
    })
}

/// Returns the snapshot the gate must apply, repairing or materializing it first when needed.
///
/// Two upgrade paths write the derived snapshot back as a payload-only guarded `Pending` update:
/// a provisional snapshot written before the actor settled, and a node that parked before
/// snapshots existed (no `payload.completion` at all). Both are re-derived only while the stored
/// snapshot still lacks a turn identity, and a repair is only accepted when it describes the same
/// user message, so a later turn can never be substituted for the one the user acted on.
fn settle_snapshot(
    repository: &SqliteWorkflowRunEngineRepository,
    node_run_id: &WorkflowNodeRunId,
    stored: Option<&CompletionSnapshot>,
    derived: CompletionSnapshot,
) -> CompletionSnapshot {
    let Some(stored) = stored else {
        // Upgrade path: the node parked before snapshots existed. Materialize once so the
        // blockers gate below applies to it exactly as it does to a new node.
        return persist_derived_snapshot(repository, node_run_id, derived);
    };
    if derived.turn_key.is_none() {
        // Nothing settled yet, so there is nothing to repair with; keep the stored snapshot.
        return stored.clone();
    }
    let same_user_message = match (stored.user_seq, derived.user_seq) {
        // A snapshot with no `userSeq` could not read history at park time, so there is no
        // identity to contradict and the repaired turn is the best available description.
        (None, _) | (_, None) => true,
        (Some(stored_seq), Some(derived_seq)) => stored_seq == derived_seq,
    };
    // A snapshot that already has a turn identity is not provisional in practice: it settled.
    if !same_user_message || (stored.turn_key.is_some() && !stored.provisional) {
        return stored.clone();
    }
    persist_derived_snapshot(repository, node_run_id, derived)
}

/// Writes the derived snapshot back and returns it, logging instead of failing on a lost guard.
///
/// The `Pending` guard is what keeps this safe outside the per-run gate: a completion or cancel
/// that already committed leaves the row outside `Pending`, so the stale snapshot is dropped.
fn persist_derived_snapshot(
    repository: &SqliteWorkflowRunEngineRepository,
    node_run_id: &WorkflowNodeRunId,
    derived: CompletionSnapshot,
) -> CompletionSnapshot {
    let patch = completion_payload(&derived);
    if let Err(error) = repository.repair_node_payload(
        node_run_id,
        &patch,
        crate::clock::SystemClock.now_timestamp_millis(),
    ) {
        ora_warn!(
            node_run_id = %node_run_id,
            error = %error,
            "failed to persist a completion snapshot while preparing a completion"
        );
    }
    derived
}

/// Maps one persisted blocker onto the contract shape carried by the rejection.
fn to_contract_blocker(blocker: &CompletionBlocker) -> WorkflowNodeCompletionBlocker {
    let kind = match blocker.kind {
        CompletionBlockerKind::TurnStopped => WorkflowNodeCompletionBlockerKind::TurnStopped,
        CompletionBlockerKind::TurnTruncated => WorkflowNodeCompletionBlockerKind::TurnTruncated,
        CompletionBlockerKind::TurnRefused => WorkflowNodeCompletionBlockerKind::TurnRefused,
        CompletionBlockerKind::TurnUnknown => WorkflowNodeCompletionBlockerKind::TurnUnknown,
        CompletionBlockerKind::TurnUnconfirmed => {
            WorkflowNodeCompletionBlockerKind::TurnUnconfirmed
        }
        CompletionBlockerKind::StructuredInvalid => {
            WorkflowNodeCompletionBlockerKind::StructuredInvalid
        }
        CompletionBlockerKind::OutputMissing => WorkflowNodeCompletionBlockerKind::OutputMissing,
    };
    WorkflowNodeCompletionBlocker {
        kind,
        message: blocker
            .params
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    }
}

/// Maps one persisted turn key onto the contract shape carried by a conflict.
fn to_contract_turn_key(key: super::super::completion_snapshot::TurnKey) -> WorkflowNodeTurnKey {
    WorkflowNodeTurnKey {
        user_seq: key.user_seq,
        ended_seq: key.ended_seq,
    }
}

/// Computes the node's own file changes from its persisted baseline and the live worktree.
///
/// A missing baseline means the node's changes cannot be attributed; reporting an empty diff is
/// safer than claiming the whole tree changed (ADR D9/E5).
pub(crate) fn completion_file_changes(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
    worktree_root: &Path,
) -> Vec<FileChange> {
    match load_worktree_baseline(baselines_root, node_run_id) {
        Some(baseline) => compute_file_changes(
            Some(&baseline),
            capture_worktree_snapshot(worktree_root).as_ref(),
        ),
        None => {
            ora_warn!(
                node_run_id = %node_run_id,
                "completing interactive node without a persisted worktree baseline; reporting no file changes"
            );
            Vec::new()
        }
    }
}

/// Loads the worktree baseline persisted when an interactive node started, or `None` when the
/// file is missing or unreadable (the completion then reports no file changes).
fn load_worktree_baseline(
    baselines_root: &Path,
    node_run_id: &WorkflowNodeRunId,
) -> Option<BTreeMap<String, Option<String>>> {
    let path = baselines_root.join(format!("{}.json", node_run_id.as_ref()));
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Maps a workflow engine repository failure onto the public backend error.
fn repository_error(source: ora_application::RepositoryError) -> BackendError {
    BackendError::from(ApplicationError::WorkflowRunRepository { source })
}

/// Rejects a completion whose snapshot no longer supports it, without exposing history details.
fn completion_rejected(public: PublicError) -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        public,
        "interactive completion rejected by its completion snapshot",
    )
}
