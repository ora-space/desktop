//! Session use cases own persisted projections, actor coordination, and workflow-aware prompts.

use crate::agent_runtime::{AgentRuntimeManager, SessionEventStream};
use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use crate::error::BackendError;
use crate::workflow::run::interactive::WorkflowSessionTurns;
use ora_application::{GetSessionHandler, ListSessionsHandler, RenameSessionHandler};
use ora_contracts::*;
use ora_db::{RepositoryPool, SqliteSessionRepository};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Executes session use cases while keeping runtime actors and workflow admission private.
///
/// Lists exclude unpublished workflow sessions, title edits commit before actor adoption and
/// notification, and prompt streams retain the workflow-owned cleanup hook when dropped.
pub struct Sessions {
    get: GetSessionHandler<SqliteSessionRepository>,
    list: ListSessionsHandler<SqliteSessionRepository>,
    rename: RenameSessionHandler<SqliteSessionRepository, SystemClock>,
    agent_runtime: Arc<AgentRuntimeManager>,
    workflow_turns: WorkflowSessionTurns,
    app_events: AppEventPublisher,
}

impl Sessions {
    /// Captures the shared runtime and restricted workflow-turn coordination from composition.
    pub(crate) fn new(
        pool: RepositoryPool,
        agent_runtime: Arc<AgentRuntimeManager>,
        workflow_turns: WorkflowSessionTurns,
        app_events: AppEventPublisher,
    ) -> Self {
        let repository = SqliteSessionRepository::new(pool);
        Self {
            get: GetSessionHandler::new(repository.clone()),
            list: ListSessionsHandler::new(repository.clone()),
            rename: RenameSessionHandler::new(repository, SystemClock),
            agent_runtime,
            workflow_turns,
            app_events,
        }
    }

    /// Creates and persists a provider session on first use.
    pub async fn start(
        &self,
        request: StartSessionRequest,
    ) -> Result<StartSessionResponse, BackendError> {
        self.agent_runtime.start_session(request).await
    }

    /// Applies one configuration option to a persisted session.
    pub async fn set_config(
        &self,
        request: SetSessionConfigRequest,
    ) -> Result<SetSessionConfigResponse, BackendError> {
        self.agent_runtime.set_session_config(request).await
    }

    /// Gets one session through the shared application composition.
    pub fn get(&self, request: GetSessionRequest) -> Result<GetSessionResponse, BackendError> {
        self.get.handle(request).map_err(BackendError::from)
    }
    /// Lists sessions through the shared application composition.
    pub fn list(&self, request: ListSessionsRequest) -> Result<ListSessionsResponse, BackendError> {
        // Snapshot unpublished ownership before reading SQLite. If a node binding commits between
        // these reads, this snapshot still excludes the row returned by the earlier database view;
        // a later request instead sees the committed binding through the repository filter.
        let unpublished = self.agent_runtime.unpublished_workflow_session_ids()?;
        let mut response = self.list.handle(request).map_err(BackendError::from)?;
        response
            .sessions
            .retain(|session| !unpublished.contains(&session.id));
        Ok(response)
    }
    /// Renames one session, locks agent title acquisition, then notifies subscribers.
    pub async fn rename(
        &self,
        request: RenameSessionRequest,
    ) -> Result<RenameSessionResponse, BackendError> {
        let session_id = request.session_id.clone();
        let response = self.rename.handle(request).map_err(BackendError::from)?;
        if let Some(title) = response.session.title.as_deref()
            && let Ok(parsed) = ora_domain::SessionTitle::parse(title)
        {
            // A missing or busy actor must not fail the rename: the row is already updated.
            let _ = self
                .agent_runtime
                .adopt_user_title(&session_id, parsed)
                .await;
        }
        self.app_events
            .try_publish(AppEvent::SessionTitleUpdated { session_id });
        Ok(response)
    }
    /// Loads one session conversation and continues its active turn when present.
    pub async fn load(
        &self,
        request: LoadSessionRequest,
    ) -> Result<SessionEventStream<LoadSessionEvent>, BackendError> {
        self.agent_runtime.load_session(request).await
    }

    /// Streams a human prompt with workflow admission and drop cleanup owned by the run module.
    pub async fn prompt(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        self.workflow_turns
            .prompt(&self.agent_runtime, request)
            .await
    }

    /// Delivers one validated permission response to the owning session actor.
    pub async fn respond_to_permission(
        &self,
        request: RespondToPermissionRequest,
    ) -> Result<RespondToPermissionResponse, BackendError> {
        self.agent_runtime.respond_to_permission(request).await
    }

    /// Unloads one running session while retaining its provider history and Ora record.
    pub async fn stop(
        &self,
        request: StopSessionRequest,
    ) -> Result<StopSessionResponse, BackendError> {
        self.agent_runtime.stop_session(request).await
    }

    /// Cancels one active prompt while keeping its session available for another turn.
    pub fn cancel_prompt(
        &self,
        request: CancelSessionPromptRequest,
    ) -> Result<CancelSessionPromptResponse, BackendError> {
        self.agent_runtime.cancel_session_prompt(request)
    }

    /// Moves one existing conversation onto a different agent CLI.
    pub async fn switch_agent(
        &self,
        request: SwitchSessionAgentRequest,
    ) -> Result<SwitchSessionAgentResponse, BackendError> {
        self.agent_runtime.switch_agent(request).await
    }

    /// Returns a session whose history writes failed to a writable state.
    pub async fn resume_history(
        &self,
        request: ResumeSessionHistoryRequest,
    ) -> Result<ResumeSessionHistoryResponse, BackendError> {
        self.agent_runtime.resume_history(request).await
    }

    /// Stops one session before removing its Ora-owned record and recorded history.
    pub async fn delete(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, BackendError> {
        self.agent_runtime.delete_session(&request.session_id).await
    }
}
