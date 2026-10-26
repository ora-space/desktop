//! Workflow-owned admission and cleanup around a session's human prompt stream.

use super::CompletingNodeRuns;
use super::session::{HumanTurnAdmission, restore_human_turn};
use crate::agent_runtime::{AgentRuntimeManager, SessionEventStream};
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use crate::workflow::run::transitions::WorkflowRunTransitions;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_contracts::{PromptSessionEvent, PromptSessionRequest};
use ora_db::RepositoryPool;
use ora_domain::PromptInactivityPolicy;
use std::path::PathBuf;
use std::sync::Arc;

/// Grants only human-turn coordination to Sessions, not workflow scheduling or completion control.
pub(crate) struct WorkflowSessionTurns {
    pool: RepositoryPool,
    run_locks: Arc<KeyedResourceLocks>,
    completing_node_runs: Arc<CompletingNodeRuns>,
    transitions: Arc<WorkflowRunTransitions>,
    sessions_root: PathBuf,
}

impl WorkflowSessionTurns {
    /// Shares the completion and callback gates owned by the run module.
    ///
    /// `sessions_root` lets the drop cleanup derive the settled turn's snapshot from the same
    /// durable history the completion path reads.
    pub(in crate::workflow::run) fn new(
        pool: RepositoryPool,
        run_locks: Arc<KeyedResourceLocks>,
        completing_node_runs: Arc<CompletingNodeRuns>,
        transitions: Arc<WorkflowRunTransitions>,
        sessions_root: PathBuf,
    ) -> Self {
        Self {
            pool,
            run_locks,
            completing_node_runs,
            transitions,
            sessions_root,
        }
    }

    /// Streams one structured ACP prompt turn for a running session.
    ///
    /// When the session belongs to an awaiting interactive workflow node, the node flips to
    /// `Running` for the duration of the turn and back to `Pending` when the turn ends or the
    /// stream is dropped, so the node's awaiting status tracks the agent's generating state. Each
    /// such turn is prefixed with the boundary of the step it belongs to, which the agent receives
    /// but the transcript does not record: the recorded user turn stays exactly what the user
    /// typed (ADR D6).
    pub(crate) async fn prompt(
        &self,
        agent_runtime: &AgentRuntimeManager,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        let admission = crate::workflow::run::interactive::begin_human_turn(
            &self.pool,
            &self.run_locks,
            &self.completing_node_runs,
            &self.transitions,
            &request.session_id,
        )
        .await?;
        let (prompt_inactivity, step_boundary) = match &admission {
            HumanTurnAdmission::Ordinary => (PromptInactivityPolicy::Timeout, None),
            HumanTurnAdmission::Workflow {
                prompt_inactivity,
                step_boundary,
                ..
            } => (*prompt_inactivity, Some(step_boundary.clone())),
        };
        let request = with_step_boundary(request, step_boundary);
        let stream = match agent_runtime
            .prompt_session_with_inactivity_policy(request, prompt_inactivity)
            .await
        {
            Ok(stream) => SessionEventStream::from(stream),
            Err(error) => {
                // The turn never started; put the awaiting node back where it was without
                // overwriting the snapshot of the turn that previously settled.
                if let HumanTurnAdmission::Workflow { node_run_id, .. } = &admission {
                    restore_human_turn(&self.transitions, node_run_id).await?;
                }
                return Err(error.into());
            }
        };
        let HumanTurnAdmission::Workflow { node_run_id, .. } = admission else {
            return Ok(stream);
        };
        let pool = self.pool.clone();
        let sessions_root = self.sessions_root.clone();
        let transitions = self.transitions.clone();
        Ok(stream.attach_cleanup(move || async move {
            crate::workflow::run::interactive::end_human_turn(
                &pool,
                &sessions_root,
                &transitions,
                &node_run_id,
            )
            .await
        }))
    }
}

/// Prepends the step boundary to the prompt the agent receives while recording the user's text.
///
/// The boundary is a separate leading content block so a provider that concatenates blocks still
/// sees it before the user's words, and `record_prompt` keeps whatever the caller already marked
/// as the recorded turn (a role expansion), falling back to the user's own prompt.
fn with_step_boundary(
    mut request: PromptSessionRequest,
    step_boundary: Option<String>,
) -> PromptSessionRequest {
    let Some(step_boundary) = step_boundary else {
        return request;
    };
    // An empty turn has nothing to scope: leave it empty so the runtime's own prompt validation
    // still rejects it instead of letting the boundary alone pass as a user message.
    if request.prompt.is_empty() {
        return request;
    }
    let recorded = request
        .record_prompt
        .clone()
        .unwrap_or_else(|| request.prompt.clone());
    request
        .prompt
        .insert(0, ContentBlock::Text(TextContent::new(step_boundary)));
    request.record_prompt = Some(recorded);
    request
}

#[cfg(test)]
mod tests {
    use super::with_step_boundary;
    use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
    use ora_contracts::PromptSessionRequest;
    use pretty_assertions::assert_eq;

    /// Builds a text content block for the boundary assertions.
    fn text(value: &str) -> ContentBlock {
        ContentBlock::Text(TextContent::new(value.to_string()))
    }

    /// Builds a prompt request with an optional recorded turn.
    fn request(
        prompt: Vec<ContentBlock>,
        record_prompt: Option<Vec<ContentBlock>>,
    ) -> PromptSessionRequest {
        PromptSessionRequest {
            session_id: "session-1".to_string(),
            prompt,
            model: None,
            record_prompt,
        }
    }

    /// The boundary is a leading block and `record_prompt` keeps the user's original text.
    #[test]
    fn boundary_precedes_the_prompt_and_records_the_original_turn() {
        let boundary = "<workflow_step_boundary>scope</workflow_step_boundary>";
        let output = with_step_boundary(
            request(vec![text("hello")], None),
            Some(boundary.to_string()),
        );
        assert_eq!(output.prompt, vec![text(boundary), text("hello")]);
        assert_eq!(output.record_prompt, Some(vec![text("hello")]));
    }

    /// An existing recorded turn (a role expansion) wins over the prompt being sent.
    #[test]
    fn boundary_preserves_an_existing_recorded_turn() {
        let output = with_step_boundary(
            request(vec![text("expanded")], Some(vec![text("original")])),
            Some("boundary".to_string()),
        );
        assert_eq!(output.prompt.first(), Some(&text("boundary")));
        assert_eq!(output.record_prompt, Some(vec![text("original")]));
    }

    /// An empty turn stays empty rather than letting the boundary alone pass as a user message.
    #[test]
    fn boundary_does_not_scope_an_empty_prompt() {
        let output = with_step_boundary(request(Vec::new(), None), Some("boundary".to_string()));
        assert!(output.prompt.is_empty());
        assert_eq!(output.record_prompt, None);
    }
}
