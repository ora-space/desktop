//! Commands serialize session operations while retaining each prompt's admission policy.

use crate::RuntimeError;
use crate::end_cause::PromptEndReport;
use crate::prompt::RecordedTurn;
use crate::title_acquisition::PollAttempt;
use agent_client_protocol_schema::v1::{ContentBlock, SessionConfigOption, SessionUpdate};
use ora_contracts::{
    LoadSessionEvent, PromptSessionEvent, RespondToPermissionRequest, RespondToPermissionResponse,
    StopSessionResponse,
};
use ora_domain::{PromptInactivityPolicy, SessionTitle};
use tokio::sync::{mpsc, oneshot};

/// Carries one owned operation or lifecycle observation to the session's serialized actor.
pub(crate) enum RuntimeCommand {
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
        commit_admission: Option<Box<dyn FnOnce() -> Result<(), RuntimeError> + Send + Sync>>,
        inactivity_policy: PromptInactivityPolicy,
        /// Applied by the attach this prompt performs, and ignored when none is needed.
        model: Option<String>,
        /// Where the actor reports why this turn ended, read by the turn's owner.
        end_report: PromptEndReport,
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
    /// Drains idle traffic and answers whether any of it belonged to a turn.
    TakeIdleTurnTraffic {
        response: oneshot::Sender<bool>,
    },
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
    /// The provider answered a direct configuration call with its options after the change.
    ///
    /// Some agents say so nowhere else. The answer is for one provider session, which may have been
    /// replaced while it was on its way. Replies with what a load answers with from here on, which
    /// is then the replacement's options rather than that answer.
    AdoptConfigOptions {
        agent_session_id: String,
        config_options: Vec<SessionConfigOption>,
        response: oneshot::Sender<Vec<SessionConfigOption>>,
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
