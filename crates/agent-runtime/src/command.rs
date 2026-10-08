//! Commands serialize session operations while retaining each prompt's admission policy.

use crate::RuntimeError;
use crate::prompt::RecordedTurn;
use crate::title_acquisition::PollAttempt;
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate};
use ora_contracts::{
    LoadSessionEvent, PromptSessionEvent, RespondToPermissionRequest, RespondToPermissionResponse,
    StopSessionResponse,
};
use ora_domain::{AgentRef, PromptInactivityPolicy, SessionTitle};
use tokio::sync::{mpsc, oneshot};

/// Carries one owned operation or lifecycle observation to the session's serialized actor.
pub(crate) enum RuntimeCommand {
    /// The agent's process was replaced, so every provider-side session it held is gone.
    ///
    /// Broadcast rather than addressed, because the actor registry is keyed by Ora session and one
    /// agent's connection is shared by every Workspace; each actor decides whether it is bound to
    /// the replaced agent.
    AgentProcessReplaced {
        agent: AgentRef,
    },
    McpDesiredMaybeChanged,
    Load {
        cleanup: oneshot::Sender<Result<(), RuntimeError>>,
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, RuntimeError>>,
        accepted: oneshot::Sender<Result<(), RuntimeError>>,
    },
    Prompt {
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        record_prompt: RecordedTurn,
        inactivity_policy: PromptInactivityPolicy,
        /// Applied by the attach this prompt performs, and ignored when none is needed.
        model: Option<String>,
        events: mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
        accepted: oneshot::Sender<Result<(), RuntimeError>>,
    },
    RespondToPermission {
        request: RespondToPermissionRequest,
        response: oneshot::Sender<Result<RespondToPermissionResponse, RuntimeError>>,
    },
    Stop {
        response: oneshot::Sender<Result<StopSessionResponse, RuntimeError>>,
    },
    CancelActivePrompt,
    Cancel {
        operation_id: u64,
        completion: Option<oneshot::Sender<Result<(), RuntimeError>>>,
    },
    /// A caller outside the actor is about to address this session's provider directly.
    ///
    /// Answers with the provider session it must address. That is not always the binding in the
    /// row: a session rebuilt to replace one that could not be restored is only persisted once the
    /// prompt carrying the transcript is accepted, so until then the actor is the only holder of
    /// the identity the agent actually answers to. Standing down title polling comes with it,
    /// because the reply is the point at which the caller starts using that identity.
    ClaimDirectProviderCall {
        response: oneshot::Sender<String>,
    },
    AdoptUserTitle {
        title: SessionTitle,
        response: oneshot::Sender<()>,
    },
    TitlePoll {
        attempt: PollAttempt,
    },
    TitleUpdate {
        update: Box<SessionUpdate>,
    },
}
