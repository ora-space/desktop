//! Orchestrates human completion of an interactive workflow node.
//!
//! The core (validating, assembling the conversation, diffing the worktree) lives here so the
//! future agent/CLI path can reuse the same commit through the engine. The session stop and the
//! engine commit are done by the caller around [`prepare_completion`].

use super::super::executor::{capture_worktree_snapshot, compute_file_changes, stop_reason_label};
use super::CompletingNodeRuns;
use crate::agent_runtime::AgentRuntimeManager;
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, StopReason};
use ora_application::{ApplicationError, FileChange, WorkflowGraph, WorkflowRunEngineRepository};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{
    SessionId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus,
};
use ora_history::{HistoryRecord, SessionHistory, read_session_history};
use ora_logging::ora_warn;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// The assembled result of validating and preparing one interactive node completion.
pub(crate) struct PreparedCompletion {
    pub node_run_id: WorkflowNodeRunId,
    pub output: Option<String>,
    pub stop_reason: Option<String>,
    pub file_changes: Vec<FileChange>,
    pub session_id: Option<SessionId>,
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
/// The run must be `Running`, the node must be awaiting input (`Pending`) and interactive, and
/// the final assistant output is read from the session's durable history. Intended to run in a
/// blocking closure so the async runtime is not held.
pub(crate) fn prepare_completion(
    pool: &RepositoryPool,
    sessions_root: &Path,
    baselines_root: &Path,
    agent_runtime: &AgentRuntimeManager,
    run_id: &WorkflowRunId,
    node_id: &str,
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
        .node(&node_run.node_id)
        .and_then(|node| node.agent_config.as_ref());
    let interactive = agent_config.is_some_and(|config| config.interactive);
    if !interactive {
        return Err(ApplicationError::WorkflowNodeNotAwaitingInput {
            node_id: node_id.to_string(),
        }
        .into());
    }
    // A `Pending` node run is always an interactive agent node, so `agent_config` is present here;
    // the default keeps the manual-completion output correct even for a malformed graph.
    let output_policy = agent_config
        .map(|config| config.output_policy)
        .unwrap_or_default();

    // The final assistant message and the stop reason of the last turn both come from the same
    // durable history, so they are read together rather than re-reading the file.
    let (output, stop_reason) = match node_run.session_id.as_ref() {
        Some(session_id) => {
            let history = read_session_history(sessions_root, session_id.as_ref())
                .map_err(|error| BackendError::internal("failed to read session history", error))?;
            (
                // Apply the node's output policy so manual completion mirrors the automatic path.
                output_policy.apply(assistant_output_from_history(&history)),
                last_stop_reason(&history).map(stop_reason_label),
            )
        }
        None => (None, None),
    };

    let file_changes = match load_worktree_baseline(baselines_root, &node_run.id) {
        Some(baseline) => {
            let worktree_root = agent_runtime.workspace_cwd(&context.workspace.id)?;
            compute_file_changes(
                Some(&baseline),
                capture_worktree_snapshot(&worktree_root).as_ref(),
            )
        }
        None => {
            // A missing baseline means the node's changes cannot be attributed; reporting an
            // empty diff is safer than claiming the whole tree changed (D9/E5).
            ora_warn!(
                node_run_id = %node_run.id,
                "completing interactive node without a persisted worktree baseline; reporting no file changes"
            );
            Vec::new()
        }
    };

    Ok(PreparedCompletion {
        node_run_id: node_run.id,
        output,
        stop_reason,
        file_changes,
        session_id: node_run.session_id,
    })
}

/// Returns the final settled assistant message from a session's durable history.
fn assistant_output_from_history(history: &SessionHistory) -> Option<String> {
    history.lines.iter().rev().find_map(|line| {
        let HistoryRecord::Update { update } = &line.record else {
            return None;
        };
        assistant_text(update.as_ref()).map(str::to_string)
    })
}

/// Returns the stop reason of the last settled turn, which is why the interactive node parked
/// awaiting input rather than completing automatically.
fn last_stop_reason(history: &SessionHistory) -> Option<StopReason> {
    history.lines.iter().rev().find_map(|line| {
        let HistoryRecord::TurnEnded { stop_reason } = &line.record else {
            return None;
        };
        Some(*stop_reason)
    })
}

/// Extracts text from one settled assistant message update.
fn assistant_text(update: &SessionUpdate) -> Option<&str> {
    let content = match update {
        SessionUpdate::AgentMessageChunk(chunk) => &chunk.content,
        _ => return None,
    };
    let ContentBlock::Text(text) = content else {
        return None;
    };
    Some(&text.text)
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

#[cfg(test)]
mod tests {
    use super::{assistant_output_from_history, last_stop_reason};
    use agent_client_protocol_schema::v1::{
        ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent,
    };
    use ora_domain::AgentRef;
    use ora_history::{HistoryIntegrity, HistoryLine, HistoryRecord, SessionHistory, SessionMeta};
    use pretty_assertions::assert_eq;

    fn turn_ended(stop_reason: StopReason) -> HistoryLine {
        HistoryLine::new(
            "2026-08-18T00:00:00+08:00",
            0,
            HistoryRecord::TurnEnded { stop_reason },
        )
    }

    fn text_update(role: &str, text: &str) -> HistoryLine {
        let update = match role {
            "user" => SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(text.to_string()),
            ))),
            _ => SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(text.to_string()),
            ))),
        };
        HistoryLine::new(
            "2026-08-18T00:00:00+08:00",
            0,
            HistoryRecord::Update {
                update: Box::new(update),
            },
        )
    }

    fn history(lines: Vec<HistoryLine>) -> SessionHistory {
        let next_seq = lines.len() as u32;
        SessionHistory {
            lines,
            next_seq,
            integrity: HistoryIntegrity::Complete,
        }
    }

    /// A multi-turn interactive node persists only its final assistant message as node output.
    #[test]
    fn assistant_output_from_history_returns_the_final_assistant_message() {
        let history = history(vec![
            text_update("user", "review the plan"),
            text_update("assistant", "here is v1"),
            text_update("user", "keep section two"),
            text_update("assistant", "v1 is final"),
        ]);
        assert_eq!(
            assistant_output_from_history(&history),
            Some("v1 is final".to_string())
        );
    }

    /// Non-message and user-only history yields no assistant node output.
    #[test]
    fn assistant_output_from_history_skips_non_assistant_records() {
        let meta = HistoryLine::new(
            "2026-08-18T00:00:00+08:00",
            0,
            HistoryRecord::Meta(SessionMeta {
                schema_version: 1,
                session_id: "session-1".to_string(),
                workspace_id: "workspace-1".to_string(),
                agent_ref: AgentRef::parse("ora-space.nga").expect("agent identity"),
                agent_session_id: "provider-1".to_string(),
                cwd: std::path::PathBuf::from("."),
            }),
        );
        assert_eq!(
            assistant_output_from_history(&history(vec![
                meta,
                text_update("user", "still working"),
            ])),
            None
        );
    }

    /// The stop reason of the last settled turn is what parked the interactive node awaiting input.
    #[test]
    fn last_stop_reason_returns_the_final_turn_stop_reason() {
        assert_eq!(
            last_stop_reason(&history(vec![turn_ended(StopReason::MaxTokens)])),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(last_stop_reason(&history(vec![])), None);
    }
}
