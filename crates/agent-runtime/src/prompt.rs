//! Admission of prompt turns onto a session's actor.
//!
//! Validation and the recorded shape of a user turn are decided here, before the actor sees the
//! command, so a prompt that could never run fails as a rejected send rather than as a turn.

use super::*;
use crate::AgentRuntimeHost;

/// What Ora records as the user turn of one prompt.
pub(crate) struct RecordedTurn {
    /// The blocks to record when they differ from the prompt the agent receives.
    pub(crate) blocks: Option<Vec<ContentBlock>>,
    /// The host's identity for this user message, written as the ACP `messageId` of every
    /// recorded user block so the file itself says which turn a message was.
    pub(crate) message_id: Option<MessageId>,
}

impl<H: AgentRuntimeHost> AgentRuntimeManager<H> {
    /// Starts one structured ACP prompt stream after validating the public payload limit.
    pub async fn prompt_session(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, RuntimeError> {
        self.send_prompt(
            request,
            /*message_id*/ None,
            PromptInactivityPolicy::Timeout,
        )
        .await
    }

    /// Starts one prompt with the host's turn-local response to inactivity.
    ///
    /// The policy is separate from the public request so only the owning host can opt a workflow
    /// turn into waiting; callers of the ordinary session API keep the default watchdog.
    pub async fn prompt_session_with_inactivity_policy(
        &self,
        request: PromptSessionRequest,
        inactivity_policy: PromptInactivityPolicy,
    ) -> Result<SessionEventStream<PromptSessionEvent>, RuntimeError> {
        self.send_prompt(request, /*message_id*/ None, inactivity_policy)
            .await
    }

    /// Starts a prompt whose recorded user message carries the host's identity for it.
    ///
    /// A host that tracks turns by its own identifiers, such as a Node relaying Cloud user turns,
    /// uses this so the history file names the turn each user message belongs to.
    pub async fn prompt_session_as_message(
        &self,
        request: PromptSessionRequest,
        message_id: MessageId,
    ) -> Result<SessionEventStream<PromptSessionEvent>, RuntimeError> {
        self.send_prompt(request, Some(message_id), PromptInactivityPolicy::Timeout)
            .await
    }

    /// Validates and admits one prompt on its session's actor.
    async fn send_prompt(
        &self,
        request: PromptSessionRequest,
        message_id: Option<MessageId>,
        inactivity_policy: PromptInactivityPolicy,
    ) -> Result<SessionEventStream<PromptSessionEvent>, RuntimeError> {
        let prompt = request.prompt;
        let record_prompt = RecordedTurn {
            blocks: request.record_prompt,
            message_id,
        };
        let model = request.model;
        if prompt.is_empty()
            || prompt.iter().all(|content| {
                matches!(content, ContentBlock::Text(text) if text.text.trim().is_empty())
            })
        {
            return Err(RuntimeError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PromptEmpty(EmptyErrorParams {}),
                "prompt must contain text or media",
            ));
        }
        let prompt_bytes = serde_json::to_vec(&prompt)
            .map_err(|error| RuntimeError::internal("failed to encode prompt", error))?
            .len();
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(RuntimeError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PromptTooLarge(EmptyErrorParams {}),
                "prompt exceeds 16 MiB",
            ));
        }
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        // Lifecycle status is not a precondition: a session that has only been read holds no
        // provider, and acquiring one is part of sending rather than something the caller has to
        // arrange first. The actor attaches, and reports its own failure if it cannot.
        //
        // A session whose history stopped recording refuses new turns rather than
        // producing conversation that would never be part of the record.
        if let HistoryState::Degraded { .. } = session.history_state {
            return Err(history_degraded());
        }
        let handle = self.actor_for(session)?;
        let operation_id = self.inner.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let (events_sender, events) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
        let (accepted_sender, accepted) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Prompt {
                operation_id,
                prompt,
                record_prompt,
                inactivity_policy,
                model,
                events: events_sender,
                accepted: accepted_sender,
            })
            .map_err(runtime_unavailable_with)?;
        accepted.await.map_err(runtime_unavailable_with)??;
        Ok(SessionEventStream::new(
            events,
            handle.commands,
            operation_id,
        ))
    }
}
