//! Workflow-owned admission and cleanup around a session's human prompt stream.

use super::super::completion_snapshot::StopCause;
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
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Session-scoped record of a prompt the user stopped, read by the park that follows it.
///
/// The cancel endpoint does not own the prompt stream, and a client that simply disconnects looks
/// the same to the stream as one whose user pressed stop. The backend records the explicit stop
/// here and the park consumes it once, so a timeout is never displayed as the user's own stop
/// (ADR D4). The marker is also cleared when a new turn starts, so it cannot explain a later turn.
#[derive(Debug, Default)]
pub(crate) struct UserPromptStops {
    sessions: Mutex<HashSet<String>>,
}

impl UserPromptStops {
    /// Builds the empty registry shared by the session use cases and their cleanup.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records that the user asked to stop this session's current prompt.
    pub(crate) fn record(&self, session_id: &str) {
        self.lock().insert(session_id.to_string());
    }

    /// Drops a stale marker before a new turn of the same session begins.
    pub(crate) fn clear(&self, session_id: &str) {
        self.lock().remove(session_id);
    }

    /// Takes the marker, so one user stop explains at most one park.
    fn take(&self, session_id: &str) -> bool {
        self.lock().remove(session_id)
    }

    /// Serializes access; a poisoned lock still holds a complete set of session ids.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Grants only human-turn coordination to Sessions, not workflow scheduling or completion control.
pub(crate) struct WorkflowSessionTurns {
    pool: RepositoryPool,
    run_locks: Arc<KeyedResourceLocks>,
    completing_node_runs: Arc<CompletingNodeRuns>,
    transitions: Arc<WorkflowRunTransitions>,
    sessions_root: PathBuf,
    /// Explicit user stops, consumed by the park of the turn they interrupted.
    user_stops: Arc<UserPromptStops>,
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
            user_stops: Arc::new(UserPromptStops::new()),
        }
    }

    /// Records the user's explicit stop so the park of this turn can classify it.
    pub(crate) fn note_user_stop(&self, session_id: &str) {
        self.user_stops.record(session_id);
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
        // A stop that belonged to an earlier turn must not explain this one.
        self.user_stops.clear(&request.session_id);
        let session_id = request.session_id.clone();
        let admission = crate::workflow::run::interactive::begin_human_turn(
            &self.pool,
            &self.run_locks,
            &self.completing_node_runs,
            &self.transitions,
            &session_id,
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
        let user_stops = self.user_stops.clone();
        Ok(stream.attach_cleanup(move || async move {
            // Only an explicit stop recorded while this turn ran becomes the park's cause; a
            // timeout keeps its own (conservative) classification instead.
            let stop_cause = user_stops
                .take(&session_id)
                .then_some(StopCause::UserStopped);
            crate::workflow::run::interactive::end_human_turn(
                &pool,
                &sessions_root,
                &transitions,
                &node_run_id,
                stop_cause,
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
    use super::{UserPromptStops, with_step_boundary};
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

    /// A recorded stop explains exactly one park and never survives a new turn.
    #[test]
    fn a_user_stop_marker_is_taken_once() {
        let stops = UserPromptStops::new();
        stops.record("session-1");
        assert!(stops.take("session-1"));
        assert!(!stops.take("session-1"));
        stops.record("session-1");
        stops.clear("session-1");
        assert!(!stops.take("session-1"));
    }
}
