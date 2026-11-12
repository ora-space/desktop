//! Orchestrates human completion of an interactive workflow node.
//!
//! The core (validating, assembling the conversation, diffing the worktree) lives here so the
//! future agent/CLI path can reuse the same commit through the engine. The session stop and the
//! engine commit are done by the caller around [`prepare_completion`].

use super::super::completion_snapshot::{
    CompletionBlocker, CompletionBlockerKind, CompletionConfirmation, CompletionMeta,
    CompletionPolicy, CompletionSnapshot, ConfirmationBy, TurnOrigin, completion_final_text,
    completion_payload, derive_completion_snapshot, forced_completion_audit,
    output_contract_blocker, parse_completion_snapshot, trailing_assistant_text,
    unknown_completion_snapshot,
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
    EmptyErrorParams, NodeCompletionRequester, PublicError, WorkflowNodeCompletionBlocker,
    WorkflowNodeCompletionBlockerKind, WorkflowNodeIncompleteParams,
    WorkflowNodeOutputChangedParams, WorkflowNodeSignalChangedParams, WorkflowNodeTurnChangedParams,
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
    /// The human approval of an accepted agent signal, committed with the completion.
    pub confirmation: Option<CompletionConfirmation>,
    /// Who the server attributes this completion to, derived from the signal path.
    pub requester: NodeCompletionRequester,
}

/// Merges the forced audit, confirmation and server-derived requester into one payload patch.
pub(crate) fn merge_completion_patch(
    audit: Option<serde_json::Value>,
    confirmation: Option<CompletionConfirmation>,
    requester: NodeCompletionRequester,
) -> Option<serde_json::Value> {
    let mut patch = audit.unwrap_or_else(|| serde_json::json!({}));
    if let Some(confirmation) = confirmation {
        patch["confirmation"] = serde_json::to_value(confirmation).ok()?;
    }
    patch["requester"] = serde_json::to_value(requester).ok()?;
    Some(patch)
}

/// What the user explicitly approved on this completion request.
///
/// Both flags are user decisions, not derived state: `confirm_initial_turn` adopts the workflow's
/// opening turn, `force` overrides a non-confirmation blocker. They are mutually exclusive at the
/// request boundary — one says the gate is satisfied, the other says it is overridden — so the
/// approval models them as named constants instead of two positional booleans (ADR D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct CompletionApproval {
    pub(crate) confirm_initial_turn: bool,
    pub(crate) force: bool,
    /// Fingerprint of the confirmed agent signal id; `None` when the request did not confirm one.
    pub(crate) confirm_agent_completion: Option<u64>,
}

/// FNV-1a fingerprint used to keep the approval `Copy` while still binding the confirmation to
/// the exact signal id the client saw.
pub(crate) fn signal_fingerprint(signal_id: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in signal_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

impl CompletionApproval {
    /// A plain completion with no override.
    pub(crate) const PLAIN: Self = Self {
        confirm_initial_turn: false,
        force: false,
        confirm_agent_completion: None,
    };
    /// Adopts the workflow's opening turn as a normal completion.
    #[cfg(test)]
    pub(crate) const CONFIRM: Self = Self {
        confirm_initial_turn: true,
        force: false,
        confirm_agent_completion: None,
    };
    /// Completes despite a non-confirmation blocker, recording the forced audit.
    #[cfg(test)]
    pub(crate) const FORCE: Self = Self {
        confirm_initial_turn: false,
        force: true,
        confirm_agent_completion: None,
    };
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

/// Rejects a combined first-turn confirmation and force before the node is claimed.
///
/// The combination is only meaningful for an `agent` policy node whose opening turn is still
/// unconfirmed and whose settlement carries no signal: confirmation lifts `turn_unconfirmed` and
/// force covers the resulting `agent_signal_pending`. Anything else has no coherent meaning and
/// must fail without changing durable state (ADR D2 row 1).
pub(crate) fn validate_first_turn_force_confirmation(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    node_id: &str,
) -> Result<(), BackendError> {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let node_run = repository
        .list_node_runs(run_id)
        .map_err(repository_error)?
        .into_iter()
        .find(|node_run| node_run.node_id == node_id)
        .ok_or_else(|| ApplicationError::WorkflowNodeNotFound {
            node_id: node_id.to_string(),
        })?;
    let meta = CompletionMeta::from_payload(node_run.payload.as_deref());
    let snapshot = parse_completion_snapshot(node_run.payload.as_deref());
    let has_unconfirmed = snapshot.as_ref().is_some_and(|snapshot| {
        snapshot
            .blockers
            .iter()
            .any(|blocker| blocker.kind == CompletionBlockerKind::TurnUnconfirmed)
    });
    if meta.policy != CompletionPolicy::Agent || !has_unconfirmed || meta.agent_signal.is_some() {
        return Err(completion_rejected(PublicError::InvalidRequest(
            EmptyErrorParams {},
        )));
    }
    Ok(())
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
    approval: CompletionApproval,
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
    let meta = CompletionMeta::from_payload(node_run.payload.as_deref());
    // A repair keeps the park's own origin and stop cause: only the turn facts are re-derived, so a
    // provisional opening turn must not become a human one and an unexplained timeout must not lose
    // its reason. A node that parked before these fields existed materializes as a human turn (ADR
    // D7), which is the conservative choice for history alone.
    let origin = stored
        .as_ref()
        .and_then(|stored| stored.turn_origin)
        .unwrap_or(TurnOrigin::Human);
    let stop_cause = stored.as_ref().and_then(|stored| stored.stop_cause);
    let (history, derived) = match node_run.session_id.as_ref() {
        Some(session_id) => {
            let history = read_session_history(sessions_root, session_id.as_ref())
                .map_err(|error| BackendError::internal("failed to read session history", error))?;
            let derived =
                derive_completion_snapshot(&history, output_contract, false, origin, stop_cause);
            (Some(history), derived)
        }
        None => (None, unknown_completion_snapshot(false, origin, stop_cause)),
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
    let stop_reason = snapshot.stop_reason.clone();
    // A forced completion without a settled turn takes the last assistant reply as its output and
    // never claims a structured output; a settled turn always uses its own contract window.
    let raw_output = match (&history, snapshot.turn_key) {
        (Some(history), Some(_)) => completion_final_text(history),
        (Some(history), None) if approval.force => trailing_assistant_text(history),
        _ => None,
    };
    // The effective blocker set joins the persisted snapshot with what this preparation recomputes
    // from the live history: a stored snapshot that looks ready must still not let the completion
    // cross an output-contract blocker the park would have recorded (ADR D2, invariant 4).
    let mut effective_blockers = snapshot.blockers.clone();
    if snapshot.turn_key.is_some()
        && let Some(blocker) = output_contract_blocker(output_contract, raw_output.as_deref())
        && !effective_blockers
            .iter()
            .any(|existing| existing.kind == blocker.kind)
    {
        effective_blockers.push(blocker);
    }
    // The agent signal gate only applies to an otherwise normally settled turn. Precise stop or
    // output blockers describe the turn itself; a signal cannot change their outcome.
    let mut agent_confirmed = false;
    if meta.policy == CompletionPolicy::Agent {
        let has_precise_blocker = effective_blockers.iter().any(|blocker| {
            matches!(
                blocker.kind,
                CompletionBlockerKind::TurnStopped
                    | CompletionBlockerKind::TurnTruncated
                    | CompletionBlockerKind::TurnRefused
                    | CompletionBlockerKind::TurnUnknown
                    | CompletionBlockerKind::StructuredInvalid
                    | CompletionBlockerKind::OutputMissing
            )
        });
        let signal_matches = snapshot.turn_key.is_some()
            && meta
                .agent_signal
                .as_ref()
                .is_some_and(|signal| Some(signal.turn_key) == snapshot.turn_key);
        if let Some(requested) = approval.confirm_agent_completion {
            match (&meta.agent_signal, signal_matches) {
                (Some(signal), true) if signal_fingerprint(&signal.signal_id) == requested => {
                    agent_confirmed = true;
                }
                _ => {
                    return Err(completion_rejected(PublicError::WorkflowNodeSignalChanged(
                        WorkflowNodeSignalChangedParams {
                            node_id: node_run.node_id.clone(),
                            expected: meta
                                .agent_signal
                                .as_ref()
                                .map(|signal| signal.signal_id.clone()),
                            actual: Some(
                                "confirmation does not match the parked agent signal".to_string(),
                            ),
                        },
                    )));
                }
            }
        }
        if !has_precise_blocker {
            if !signal_matches {
                effective_blockers
                    .push(CompletionBlocker::new(CompletionBlockerKind::AgentSignalPending));
            } else if meta.confirmation.is_none() && !agent_confirmed {
                effective_blockers.push(CompletionBlocker::new(
                    CompletionBlockerKind::AgentConfirmationPending,
                ));
            }
        }
    }
    // `confirm_initial_turn` only lifts `turn_unconfirmed`; every other blocker still applies and
    // `force` cannot stand in for confirmation. What remains is exactly what the user has not
    // resolved, so a rejection lists only those blockers (ADR D2 rows 5–10).
    let unfulfilled: Vec<CompletionBlocker> = effective_blockers
        .iter()
        .filter(|blocker| {
            !(approval.confirm_initial_turn
                && blocker.kind == CompletionBlockerKind::TurnUnconfirmed)
                && !(agent_confirmed
                    && blocker.kind == CompletionBlockerKind::AgentConfirmationPending)
        })
        .cloned()
        .collect();
    // The combined first-turn exception is only valid when a `turn_unconfirmed` is actually
    // lifted and at least one other blocker remains for `force` to cover.
    if approval.force && approval.confirm_initial_turn {
        let had_unconfirmed = effective_blockers
            .iter()
            .any(|blocker| blocker.kind == CompletionBlockerKind::TurnUnconfirmed);
        if !had_unconfirmed || unfulfilled.is_empty() {
            return Err(completion_rejected(PublicError::InvalidRequest(
                EmptyErrorParams {},
            )));
        }
    }
    let forced = !unfulfilled.is_empty()
        && approval.force
        && !unfulfilled
            .iter()
            .any(|blocker| blocker.kind == CompletionBlockerKind::TurnUnconfirmed);
    if !unfulfilled.is_empty() && !forced {
        return Err(incomplete(&node_run.node_id, &unfulfilled));
    }
    let (output, structured_output) =
        match apply_output_contract(output_contract, raw_output.clone(), &node_run.node_id) {
            Ok(applied) => applied,
            Err(_) if forced => (raw_output, None),
            Err(_) => return Err(incomplete(&node_run.node_id, &unfulfilled)),
        };

    // Confirmation is a normal completion fact, so it is committed beside the snapshot; only a
    // forced completion uses the audit instead (ADR D5).
    let confirmation = agent_confirmed.then(|| CompletionConfirmation {
        by: ConfirmationBy::Human,
        at: (ora_logging::clock::now_local().unix_timestamp_nanos() / 1_000_000) as i64,
    });
    // The requester is a server fact: only a settled agent signal attributes the completion to the
    // agent; every other accepted path remains a human completion.
    let requester = if meta.agent_signal.is_some() {
        NodeCompletionRequester::Agent
    } else {
        NodeCompletionRequester::Human
    };

    Ok(PreparedCompletion {
        node_run_id: node_run.id,
        output,
        structured_output,
        stop_reason,
        session_id: node_run.session_id,
        workspace_id: context.workspace.id,
        audit: forced.then(|| forced_completion_audit(&snapshot, &unfulfilled)),
        confirmation,
        requester,
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
        CompletionBlockerKind::AgentSignalPending => {
            WorkflowNodeCompletionBlockerKind::AgentSignalPending
        }
        CompletionBlockerKind::AgentConfirmationPending => {
            WorkflowNodeCompletionBlockerKind::AgentConfirmationPending
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

/// Rejects a completion whose effective blockers the approval does not satisfy.
///
/// Only the blockers the request has not resolved are listed: a confirmed opening turn no longer
/// appears, while every other stored or recomputed blocker still does (ADR D2 row 9).
fn incomplete(node_id: &str, blockers: &[CompletionBlocker]) -> BackendError {
    completion_rejected(PublicError::WorkflowNodeIncomplete(
        WorkflowNodeIncompleteParams {
            node_id: node_id.to_string(),
            blockers: blockers.iter().map(to_contract_blocker).collect(),
        },
    ))
}

/// Rejects a completion whose snapshot no longer supports it, without exposing history details.
fn completion_rejected(public: PublicError) -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        public,
        "interactive completion rejected by its completion snapshot",
    )
}
