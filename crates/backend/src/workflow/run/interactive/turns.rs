//! Workflow-owned admission and cleanup around a session's human prompt stream.

use super::super::completion_snapshot::{CompletionPolicy, StopCause};
use super::super::operations::WorkflowRuns;
use super::CompletingNodeRuns;
use super::agent_signal::AgentSignalStore;
use super::session::{HumanTurnAdmission, restore_human_turn};
use crate::agent_runtime::{AgentRuntimeManager, SessionEventStream};
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use crate::workflow::run::transitions::WorkflowRunTransitions;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_agent_runtime::PromptEndReport;
use ora_contracts::{PromptSessionEvent, PromptSessionRequest};
use ora_db::RepositoryPool;
use ora_domain::{PromptInactivityPolicy, WorkflowNodeRunId};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Grants only human-turn coordination to Sessions, not workflow scheduling or completion control.
pub(crate) struct WorkflowSessionTurns {
    pool: RepositoryPool,
    run_locks: Arc<KeyedResourceLocks>,
    completing_node_runs: Arc<CompletingNodeRuns>,
    transitions: Arc<WorkflowRunTransitions>,
    sessions_root: PathBuf,
    /// Per-attempt tokens shared with the host completion MCP server.
    agent_signals: Arc<AgentSignalStore>,
    /// Weak owner used to run the headless auto-accept after a settled human turn parks.
    auto_completer: Option<std::sync::Weak<WorkflowRuns>>,
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
        agent_signals: Arc<AgentSignalStore>,
        auto_completer: Option<std::sync::Weak<WorkflowRuns>>,
    ) -> Self {
        Self {
            pool,
            run_locks,
            completing_node_runs,
            transitions,
            sessions_root,
            agent_signals,
            auto_completer,
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
        let session_id = request.session_id.clone();
        let admission = crate::workflow::run::interactive::begin_human_turn(
            &self.pool,
            &self.run_locks,
            &self.completing_node_runs,
            &self.transitions,
            &session_id,
        )
        .await?;
        let (prompt_inactivity, step_boundary, completion_policy, auto_accept) = match &admission {
            HumanTurnAdmission::Ordinary => (
                PromptInactivityPolicy::Timeout,
                None,
                CompletionPolicy::EndTurn,
                false,
            ),
            HumanTurnAdmission::Workflow {
                prompt_inactivity,
                step_boundary,
                completion_policy,
                auto_accept,
                ..
            } => (
                *prompt_inactivity,
                Some(step_boundary.clone()),
                *completion_policy,
                *auto_accept,
            ),
        };
        // The agent policy needs one token per prompt attempt. It rides on the step boundary so
        // the recorded user message stays exactly what the user typed.
        let (step_boundary, attempt_id) = match (&admission, completion_policy) {
            (HumanTurnAdmission::Workflow { node_run_id, .. }, CompletionPolicy::Agent) => {
                let attempt = uuid::Uuid::new_v4().to_string();
                let token = self
                    .agent_signals
                    .issue(&format!("{node_run_id}"), &attempt);
                let boundary = step_boundary.unwrap_or_default();
                (
                    Some(format!(
                        "{boundary}\n<workflow_completion>\nWhen this step is complete, call the \
                         tool ora_complete_workflow_step and pass this attemptToken unchanged: \
                         {token}\n</workflow_completion>"
                    )),
                    Some(attempt),
                )
            }
            _ => (step_boundary, None),
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
                    if let Some(attempt) = attempt_id.as_deref() {
                        self.agent_signals
                            .revoke_attempt(&format!("{node_run_id}"), attempt);
                    }
                    restore_human_turn(&self.transitions, node_run_id).await?;
                }
                return Err(error.into());
            }
        };
        let HumanTurnAdmission::Workflow { node_run_id, .. } = admission else {
            return Ok(stream);
        };
        let end_report = stream.end_report();
        let pool = self.pool.clone();
        let sessions_root = self.sessions_root.clone();
        let transitions = self.transitions.clone();
        let agent_signals = self.agent_signals.clone();
        let auto_completer = self.auto_completer.clone();
        Ok(stream.attach_cleanup(move || async move {
            // The runtime attributes this turn's end while it settles: an explicit stop, the
            // inactivity policy, a dropped stream and a crashed session all end it as `Cancelled`
            // in history, so the park must be told which one it was (ADR D4).
            park_ended_turn(
                &pool,
                &sessions_root,
                &transitions,
                &node_run_id,
                end_report,
                completion_policy,
                auto_accept,
                Some(agent_signals),
                attempt_id,
            )
            .await?;
            if auto_accept {
                if let Some(runs) = auto_completer.and_then(|weak| weak.upgrade()) {
                    // Run outside the cleanup future: completing the node stops its session, and
                    // waiting for that stop from inside the stream's own cleanup could deadlock.
                    let node_run_id = node_run_id.clone();
                    tokio::spawn(async move {
                        let _ = runs.auto_complete_node(&node_run_id).await;
                    });
                }
            }
            Ok(())
        }))
    }
}

/// Parks one workflow-owned turn with the cause the runtime reported for it.
///
/// Cleanup runs after the stream's owner settled, which is when the actor's report is final. The
/// report is the only place the end is attributable: history records a user stop, an inactivity
/// timeout, a dropped stream and a crash all as the same `Cancelled` stop reason.
#[allow(clippy::too_many_arguments)]
pub(super) async fn park_ended_turn(
    pool: &RepositoryPool,
    sessions_root: &Path,
    transitions: &Arc<WorkflowRunTransitions>,
    node_run_id: &WorkflowNodeRunId,
    end_report: Option<PromptEndReport>,
    completion_policy: CompletionPolicy,
    auto_accept: bool,
    agent_signals: Option<Arc<AgentSignalStore>>,
    attempt_id: Option<String>,
) -> Result<(), BackendError> {
    let stop_cause = StopCause::of_prompt_end(end_report.and_then(|report| report.recorded()));
    crate::workflow::run::interactive::end_human_turn_with_signal(
        pool,
        sessions_root,
        transitions,
        node_run_id,
        stop_cause,
        completion_policy,
        auto_accept,
        agent_signals,
        attempt_id,
    )
    .await
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
