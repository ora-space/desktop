//! Commands serialize session operations while retaining each prompt's admission policy.

use crate::RuntimeError;
use crate::config_changes::ConfigChangeRoute;
use crate::end_cause::PromptEndReport;
use crate::prompt::RecordedTurn;
use crate::title_acquisition::PollAttempt;
use agent_client_protocol_schema::v1::{
    ContentBlock, SessionConfigId, SessionConfigOption, SessionConfigOptionValue, SessionUpdate,
};
use ora_contracts::{
    LoadSessionEvent, PromptSessionEvent, RespondToPermissionRequest, RespondToPermissionResponse,
    StopSessionResponse,
};
use ora_domain::{PromptInactivityPolicy, SessionTitle};
use tokio::sync::{mpsc, oneshot};

/// One option a session's provider is to be asked for.
pub(crate) struct ConfigChangeRequest {
    pub(crate) config_id: SessionConfigId,
    pub(crate) value: SessionConfigOptionValue,
    /// Told how the change is on its way once the actor has asked.
    pub(crate) accepted: oneshot::Sender<Result<ConfigChangeRoute, RuntimeError>>,
}

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
    /// Answers the options the session is on, or `None` when it holds no provider.
    ///
    /// Read-only and answered wherever commands are read, so a view asking during a turn, a title
    /// poll or an MCP refresh is not kept waiting for it.
    ReadOptions {
        response: oneshot::Sender<Option<Vec<SessionConfigOption>>>,
    },
    /// Drains idle traffic and answers whether any of it belonged to a turn.
    TakeIdleTurnTraffic {
        response: oneshot::Sender<bool>,
    },
    Cancel {
        operation_id: u64,
        completion: Option<oneshot::Sender<Result<(), RuntimeError>>>,
    },
    /// Asks this session's provider for one option.
    ///
    /// The actor asks rather than its caller, so that the answer arrives among the session's other
    /// frames and is addressed to the provider session it is talking to right now. That is not always
    /// the binding in the row: a session rebuilt to replace one that could not be restored is only
    /// persisted once the prompt carrying the transcript is accepted. Title polling stands down.
    ChangeConfig(ConfigChangeRequest),
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
