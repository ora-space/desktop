//! Workflow-owned admission and cleanup around a session's human prompt stream.

use super::CompletingNodeRuns;
use crate::agent_runtime::{AgentRuntimeManager, SessionEventStream};
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use ora_contracts::{PromptSessionEvent, PromptSessionRequest};
use ora_db::RepositoryPool;
use std::sync::Arc;

/// Grants only human-turn coordination to Sessions, not workflow scheduling or completion control.
pub(crate) struct WorkflowSessionTurns {
    pool: RepositoryPool,
    run_locks: Arc<KeyedResourceLocks>,
    completing_node_runs: Arc<CompletingNodeRuns>,
}

impl WorkflowSessionTurns {
    /// Shares the completion and callback gates owned by the run module.
    pub(in crate::workflow::run) fn new(
        pool: RepositoryPool,
        run_locks: Arc<KeyedResourceLocks>,
        completing_node_runs: Arc<CompletingNodeRuns>,
    ) -> Self {
        Self {
            pool,
            run_locks,
            completing_node_runs,
        }
    }

    /// Streams one structured ACP prompt turn for a running session.
    ///
    /// When the session belongs to an awaiting interactive workflow node, the node flips to
    /// `Running` for the duration of the turn and back to `Pending` when the turn ends or the
    /// stream is dropped, so the node's awaiting status tracks the agent's generating state.
    pub(crate) async fn prompt(
        &self,
        agent_runtime: &AgentRuntimeManager,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        let node_run_id = crate::workflow::run::interactive::begin_human_turn(
            &self.pool,
            &self.run_locks,
            &self.completing_node_runs,
            &request.session_id,
        )
        .await?;
        let stream = match agent_runtime.prompt_session(request).await {
            Ok(stream) => stream,
            Err(error) => {
                // The turn never started; put the awaiting node back where it was.
                if let Some(node_run_id) = node_run_id.as_ref() {
                    let _ =
                        crate::workflow::run::interactive::end_human_turn(&self.pool, node_run_id)
                            .await;
                }
                return Err(error);
            }
        };
        let Some(node_run_id) = node_run_id else {
            return Ok(stream);
        };
        let pool = self.pool.clone();
        Ok(stream.attach_cleanup(move || {
            tokio::spawn(async move {
                let _ =
                    crate::workflow::run::interactive::end_human_turn(&pool, &node_run_id).await;
            });
        }))
    }
}
