mod actor;
mod connection;
mod events;
mod handoff;
mod history;
pub(crate) mod plugin_agent;
mod replay;
mod restart_circuit;
mod routing;
mod scheduling;
mod session_followers;
mod stream;
mod support;
mod title_acquisition;
mod warm;
mod warm_pool;

#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod replaced_sessions_tests;

use crate::app_event::AppEventPublisher;
use history::{LocalHistoryClock, RecordOutcome, SessionRecorder};
pub use stream::SessionEventStream;
use support::*;
use title_acquisition::TitleAcquisition;

use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::task::resolve_workspace_cwd;
use crate::{BackendError, ErrorClassification};
use agent_client_protocol_schema::v1::AvailableCommand;
use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use agent_client_protocol_schema::v1::{SessionConfigId, SessionConfigOptionValue};
use connection::{ConnectionStatus, ConnectionSupervisor, ConnectionSupervisors};
use ora_application::{Clock, SessionRepository};
use ora_contracts::{
    AttachSessionRequest, AttachSessionResponse, CancelSessionPromptRequest,
    CancelSessionPromptResponse, DeleteSessionResponse, LoadSessionEvent, LoadSessionRequest,
    PromptSessionEvent, PromptSessionRequest, RespondToPermissionRequest,
    RespondToPermissionResponse, ResumeSessionHistoryRequest, ResumeSessionHistoryResponse,
    SetSessionConfigRequest, SetSessionConfigResponse, StopSessionRequest, StopSessionResponse,
    SwitchSessionAgentRequest, SwitchSessionAgentResponse, WarmSessionRequest, WarmSessionResponse,
    WarmSessionTarget,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{
    AgentRef, AuditFields, HistoryState, PluginId, Session, SessionId, SessionStatus, SessionTitle,
    WorkspaceId,
};
use ora_history::{HistoryIntegrity, binding_needs_handoff, read_session_history};
use ora_logging::{ora_debug, ora_warn};
use ora_scheduler::Scheduler;
use routing::{SessionChannel, SessionEvent};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use warm::WarmSessions;
use warm_pool::WarmKey;

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
const SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const CANCELLATION_GRACE: Duration = Duration::from_secs(5);
const CONTRACT_QUEUE_CAPACITY: usize = 256;
const MAX_PROMPT_BYTES: usize = 16 * 1024 * 1024;

/// Identifies the internal owner of a warm provider session.
///
/// Interactive sessions are single-window Desktop state. Workflow nodes keep
/// their run and node identity so concurrent graph branches cannot claim one
/// another's configured provider session without exposing that ownership in a
/// frontend contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum WarmOwner {
    Interactive,
    WorkflowNode { run_id: String, node_id: String },
}

/// Repairs live sessions after the agent process behind them was replaced.
///
/// Effect coordination restarts an Agent plugin's process so it re-reads a materialized surface,
/// which silently invalidates every provider-side session that process was holding. Implementations
/// detach those sessions so the next interaction re-establishes them through the ordinary load path
/// rather than prompting against an id the fresh process cannot resolve.
///
/// The Effect worker depends on this capability rather than on the whole agent runtime, so a
/// reconcile stays exercisable without one.
pub(crate) trait ReplacedAgentSessions: Send + Sync + 'static {
    /// Detaches every live session served by the plugin whose agent process was replaced.
    ///
    /// Takes the package address deliberately, rather than the `AgentRef` a Session is bound to. A
    /// plugin carries both identities and Effect only ever holds this one; accepting the other
    /// would let a caller pass an address where an agent name belongs, which compiles, reads
    /// correctly, and silently matches no session at all.
    fn detach_sessions_for_replaced_plugin(&self, plugin_id: &PluginId);
}

impl ReplacedAgentSessions for AgentRuntimeManager {
    /// One agent's connection is shared by every Workspace, so replacing that process invalidates
    /// sessions well beyond the Workspace whose surface was reconciled. The command is broadcast
    /// and each actor decides whether it is bound to this agent, because the registry is keyed by
    /// Ora session and carries no agent index. Delivery is best effort: an actor that already
    /// ended cannot be holding a stale channel either.
    fn detach_sessions_for_replaced_plugin(&self, plugin_id: &PluginId) {
        let Some(agent) = self.inner.connections.agent_for_plugin(plugin_id) else {
            // Only an agent-contributing package can have had sessions to lose, so a package that
            // resolves to no agent identity is not a failure — but it is worth saying, because the
            // alternative reading is that the translation itself broke.
            ora_debug!(
                plugin_id = %plugin_id,
                "replaced plugin contributes no agent identity; no session to detach",
            );
            return;
        };
        let Ok(actors) = self.inner.actors.read() else {
            // A poisoned registry means an actor panicked; whatever it held is not trustworthy,
            // and the next interaction rebuilds the session from durable state regardless.
            return;
        };
        for handle in actors.values() {
            let _ = handle.commands.send(RuntimeCommand::AgentProcessReplaced {
                agent: agent.clone(),
            });
        }
    }
}

/// Coordinates one serialized actor per Ora session on its selected supervised CLI connection.
#[derive(Clone)]
pub(crate) struct AgentRuntimeManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    pool: RepositoryPool,
    actors: RwLock<HashMap<SessionId, RuntimeActorHandle>>,
    /// Workflow sessions stay unpublished here until their durable node-run binding exists.
    unpublished_workflow_sessions: RwLock<HashSet<SessionId>>,
    lifecycle: tokio::sync::Mutex<()>,
    next_operation_id: AtomicU64,
    connections: ConnectionSupervisors,
    sessions_root: PathBuf,
    warm: WarmSessions,
    clock: SystemClock,
    scheduler: Scheduler,
    app_events: AppEventPublisher,
    relative_path_base: PathBuf,
}

#[derive(Clone)]
struct RuntimeActorHandle {
    commands: mpsc::UnboundedSender<RuntimeCommand>,
}

pub(super) enum RuntimeCommand {
    /// The agent's process was replaced, so every provider-side session it held is gone.
    ///
    /// Broadcast rather than addressed, because the actor registry is keyed by Ora session and one
    /// agent's connection is shared by every Workspace; each actor decides whether it is bound to
    /// the replaced agent.
    AgentProcessReplaced {
        agent: AgentRef,
    },
    Load {
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
        accepted: oneshot::Sender<Result<(), BackendError>>,
    },
    Prompt {
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        record_prompt: Option<Vec<ContentBlock>>,
        events: mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
        accepted: oneshot::Sender<Result<(), BackendError>>,
    },
    RespondToPermission {
        request: RespondToPermissionRequest,
        response: oneshot::Sender<Result<RespondToPermissionResponse, BackendError>>,
    },
    Stop {
        response: oneshot::Sender<Result<StopSessionResponse, BackendError>>,
    },
    CancelActivePrompt,
    Cancel {
        operation_id: u64,
    },
    PreemptTitlePolling {
        response: oneshot::Sender<()>,
    },
    AdoptUserTitle {
        title: SessionTitle,
        response: oneshot::Sender<()>,
    },
    TitlePoll {
        attempt: title_acquisition::PollAttempt,
    },
    TitleUpdate {
        update: Box<SessionUpdate>,
    },
}

struct RuntimeActor {
    session: Session,
    cwd: PathBuf,
    repository: SqliteSessionRepository,
    clock: SystemClock,
    connection: ConnectionSupervisor,
    channel: Option<SessionChannel>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    recorder: SessionRecorder,
    sessions_root: PathBuf,
    /// Whether the current provider binding still has to be told the history.
    ///
    /// Switching agents rebinds eagerly but injects lazily, so this is answered
    /// from the record when the actor opens and cleared once a prompt carries the
    /// transcript across.
    handoff_pending: bool,
    scheduler: Scheduler,
    app_events: AppEventPublisher,
    title_acquisition: TitleAcquisition,
    command_sender: mpsc::WeakUnboundedSender<RuntimeCommand>,
    #[cfg(test)]
    exit_probe: Option<oneshot::Sender<()>>,
}

/// One session's opened recorder together with what reading its file revealed.
struct OpenedRecorder {
    recorder: SessionRecorder,
    handoff_pending: bool,
    /// Set when the history could not be read, which degrades the session.
    ///
    /// A history Ora cannot read is one it cannot safely extend: appending
    /// without knowing the positions already used would overwrite them.
    failure: Option<String>,
}

/// Groups the fixed dependencies the agent runtime is constructed from.
pub(crate) struct AgentRuntimeSetup {
    /// Owns the processes behind plugin-provided agents and the set of installed packages.
    pub plugin_host: Arc<PluginApi>,
    pub pool: RepositoryPool,
    pub home_directory: PathBuf,
    pub relative_path_base: PathBuf,
    pub sessions_root: PathBuf,
    pub clock: SystemClock,
    pub scheduler: Scheduler,
    pub app_events: AppEventPublisher,
}

impl AgentRuntimeManager {
    /// Builds the manager, reconciles stale rows, and immediately starts the shared supervisor.
    pub(crate) fn new(setup: AgentRuntimeSetup) -> Result<Self, BackendError> {
        let AgentRuntimeSetup {
            plugin_host,
            pool,
            home_directory,
            relative_path_base,
            sessions_root,
            clock,
            scheduler,
            app_events,
        } = setup;
        reconcile_running_sessions(&pool, clock)?;
        let connections =
            ConnectionSupervisors::start(plugin_host, pool.clone(), home_directory, clock);
        Ok(Self {
            inner: Arc::new(ManagerInner {
                pool,
                actors: RwLock::new(HashMap::new()),
                unpublished_workflow_sessions: RwLock::new(HashSet::new()),
                lifecycle: tokio::sync::Mutex::new(()),
                next_operation_id: AtomicU64::new(1),
                warm: WarmSessions::new(connections.clone(), clock),
                connections,
                sessions_root,
                clock,
                scheduler,
                app_events,
                relative_path_base,
            }),
        })
    }

    /// Returns the warm provider session backing one chat surface.
    ///
    /// This deliberately avoids the lifecycle lock. Opening a chat surface is a
    /// navigation-frequency operation now that every surface warms a session,
    /// and serializing it against prompts would make browsing the workspace stall
    /// unrelated conversations.
    pub(crate) async fn warm_session(
        &self,
        request: WarmSessionRequest,
    ) -> Result<WarmSessionResponse, BackendError> {
        self.warm_session_for_owner(request, WarmOwner::Interactive)
            .await
    }

    /// Returns a warm provider session for an explicitly owned internal workflow surface.
    pub(crate) async fn warm_session_for_owner(
        &self,
        request: WarmSessionRequest,
        owner: WarmOwner,
    ) -> Result<WarmSessionResponse, BackendError> {
        let agent_ref = domain_agent_ref(request.agent_ref)?;
        let workspace_id = self.resolve_warm_workspace_id(&request.target)?;
        let cwd = self.resolve_warm_cwd(&request.target)?;
        let key = WarmKey {
            target: request.target,
            agent_ref,
            owner,
        };
        let (session_id, config_options) = self.inner.warm.warm(key, cwd).await?;
        Ok(WarmSessionResponse {
            session_id: session_id.to_string(),
            workspace_id: workspace_id.to_string(),
            config_options,
        })
    }

    /// Brings the supervised agent set in line with the plugin packages installed right now.
    ///
    /// Every plugin operation that changes which packages exist calls this, so a plugin installed
    /// or removed while Ora runs is reflected in the agent picker and in session routing without a
    /// restart.
    pub(crate) fn sync_plugin_agents(&self) {
        self.inner.connections.sync_plugin_agents();
    }

    /// Reports the models one agent advertises before any session exists.
    ///
    /// The list is whatever the agent published when its current connection came up. An agent
    /// that has no pre-session model list returns an empty one rather than an error, because
    /// "this agent does not advertise models" is a normal answer for built-in CLIs.
    pub(crate) fn agent_models(
        &self,
        request: ora_contracts::ListAgentModelsRequest,
    ) -> Result<ora_contracts::ListAgentModelsResponse, BackendError> {
        let supervisor = self
            .inner
            .connections
            .for_agent(&domain_agent_ref(request.agent_ref)?)?;
        let connection = supervisor.current()?;
        Ok(ora_contracts::ListAgentModelsResponse {
            models: connection
                .models
                .iter()
                .map(|model| ora_contracts::AgentModel {
                    id: model.id.clone(),
                    display_name: model.display_name.clone(),
                    default: model.default,
                })
                .collect(),
        })
    }

    /// Reports the live ACP handshake status of every supervised agent runtime.
    ///
    /// The set is whatever this installation actually supervises, not a fixed list: an agent
    /// contributed by a plugin appears here exactly like a built-in one.
    pub(crate) fn agent_runtime_status(&self) -> ora_contracts::GetAgentRuntimeStatusResponse {
        ora_contracts::GetAgentRuntimeStatusResponse {
            statuses: self
                .inner
                .connections
                .statuses()
                .into_iter()
                .map(|(agent_ref, status)| ora_contracts::AgentRuntimeStatus {
                    agent_ref: agent_ref.into(),
                    status: match status {
                        ConnectionStatus::Ready => ora_contracts::AgentStatus::Ready,
                        ConnectionStatus::Starting => ora_contracts::AgentStatus::Starting,
                        ConnectionStatus::Unavailable => ora_contracts::AgentStatus::Unavailable,
                        ConnectionStatus::Failing => ora_contracts::AgentStatus::Failing,
                    },
                })
                .collect(),
        }
    }

    /// Discards the warm sessions belonging to chat surfaces that were deleted.
    ///
    /// Called when a Task or project goes away. Those surfaces can never be
    /// requested again, so this is the only point at which their provider
    /// sessions are returned to the agent.
    pub(crate) async fn discard_warm_sessions(&self, targets: &[WarmSessionTarget]) {
        self.inner.warm.discard(targets).await;
    }

    /// Applies one configuration option to a warm or persisted session.
    pub(crate) async fn set_session_config(
        &self,
        request: SetSessionConfigRequest,
    ) -> Result<SetSessionConfigResponse, BackendError> {
        let session_id = SessionId::new(request.session_id.as_str());
        let config_id = SessionConfigId::new(request.config_id);
        let value = SessionConfigOptionValue::value_id(request.value);
        if let Some(result) = self
            .inner
            .warm
            .set_config(&session_id, config_id.clone(), value.clone())
            .await
        {
            return result.map(|config_options| SetSessionConfigResponse { config_options });
        }
        // Not warm, so this is a persisted session whose actor owns its stream.
        let session = self.find_session(&request.session_id)?;
        if let Some(handle) = self.lookup_actor(&session.id)? {
            let (response, acknowledged) = oneshot::channel();
            handle
                .commands
                .send(RuntimeCommand::PreemptTitlePolling { response })
                .map_err(|_error| runtime_unavailable())?;
            acknowledged.await.map_err(|_error| runtime_unavailable())?;
        }
        // The provider request remains direct because it is independent of the actor's
        // serialized prompt/load stream; only the title-polling attempt needs preemption.
        let config_options = warm::request_config_option(
            &self.inner.connections,
            &session.agent_ref,
            &session.agent_session_id,
            &config_id,
            &value,
        )
        .await?;
        Ok(SetSessionConfigResponse { config_options })
    }

    /// Locks first-title acquisition so a later agent title cannot overwrite a user rename.
    ///
    /// Missing actors are a no-op: restored sessions already start with acquisition disabled.
    pub(crate) async fn adopt_user_title(
        &self,
        session_id: &str,
        title: SessionTitle,
    ) -> Result<(), BackendError> {
        let session_id = SessionId::new(session_id);
        let Some(handle) = self.lookup_actor(&session_id)? else {
            return Ok(());
        };
        let (response, acknowledged) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::AdoptUserTitle { title, response })
            .map_err(|_error| runtime_unavailable())?;
        acknowledged.await.map_err(|_error| runtime_unavailable())
    }

    /// Persists one warm session against the Workspace that owns it.
    ///
    /// `warm.take` only reserves the warm session; it stays in the pool until
    /// `commit` below, which runs on the one path where the session is durably
    /// persisted. Every other way out of this function — an error, or a caller
    /// dropped mid-await — drops the reservation instead, which returns the
    /// entry to the pool rather than stranding the client's id or pinning the
    /// provider session behind it.
    pub(crate) async fn attach_session(
        &self,
        request: AttachSessionRequest,
    ) -> Result<AttachSessionResponse, BackendError> {
        let session_id = SessionId::new(request.session_id.as_str());
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let cwd = self.workspace_cwd(&workspace_id)?;
        // The provider handshake a rebuild may need runs before the lifecycle
        // lock is taken, so attaching never blocks other sessions on the network.
        let reservation = self.inner.warm.take(&session_id, &cwd).await?;
        let attachment = reservation.attachment();
        let agent_ref = attachment.agent_ref.clone();
        let agent_session_id = attachment.agent_session_id.clone();
        let session_cwd = attachment.cwd.clone();
        let available_commands = attachment.available_commands.clone();

        let response = async {
            let _lifecycle = self.inner.lifecycle.lock().await;
            let supervisor = self.inner.connections.for_agent(&agent_ref)?;
            let channel =
                supervisor.open_session_channel(&agent_session_id, session_id.as_ref())?;
            let now = self.inner.clock.now_timestamp_millis();
            let session = Session::new(
                session_id.clone(),
                workspace_id,
                agent_ref,
                agent_session_id,
                SessionStatus::Running,
                AuditFields::new(now, now, false),
            );
            SqliteSessionRepository::new(self.inner.pool.clone())
                .create_session(session.clone())
                .map_err(|source| {
                    BackendError::internal("failed to persist agent CLI session", source)
                })?;
            ora_debug!(
                session_id = %session.id,
                agent_session_id = %session.agent_session_id,
                "warm session attached",
            );
            // The header opens the file this conversation owns for the rest of its
            // life, so it is written before the session can be prompted.
            let mut opened = self.open_recorder(&session)?;
            let outcome = match opened.failure.take() {
                Some(reason) => RecordOutcome::JustFailed { reason },
                None => opened.recorder.record_meta(&session, &session_cwd),
            };
            let session = self.settle_record(session, outcome);
            let title_acquisition =
                TitleAcquisition::awaiting_first_prompt(channel.connection.list_session_supported);
            self.insert_actor(
                session.clone(),
                ActorSetup {
                    cwd: session_cwd,
                    connection: supervisor,
                    channel: Some(channel),
                    recorder: opened.recorder,
                    handoff_pending: false,
                    title_acquisition,
                },
            )?;
            Ok::<_, BackendError>(AttachSessionResponse {
                session: contract_session(session),
                available_commands,
            })
        }
        .await?;

        reservation.commit();
        Ok(response)
    }

    /// Attaches a workflow node Session while keeping it out of ordinary list snapshots.
    pub(crate) async fn attach_workflow_node_session(
        &self,
        request: AttachSessionRequest,
    ) -> Result<AttachSessionResponse, BackendError> {
        let session_id = SessionId::new(request.session_id.as_str());
        self.unpublished_workflow_sessions_write()?
            .insert(session_id.clone());
        let result = self.attach_session(request).await;
        if result.is_err() {
            self.unpublished_workflow_sessions_write()?
                .remove(&session_id);
        }
        result
    }

    /// Captures workflow Sessions whose durable node-run binding is not visible yet.
    pub(crate) fn unpublished_workflow_session_ids(&self) -> Result<HashSet<String>, BackendError> {
        self.inner
            .unpublished_workflow_sessions
            .read()
            .map(|sessions| sessions.iter().map(ToString::to_string).collect())
            .map_err(|_poisoned| runtime_unavailable())
    }

    /// Publishes a workflow Session only after its node-run binding has committed.
    pub(crate) fn publish_workflow_node_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), BackendError> {
        self.unpublished_workflow_sessions_write()?
            .remove(session_id);
        Ok(())
    }

    /// Removes a workflow Session whose setup failed before any node-run binding was published.
    pub(crate) async fn discard_unpublished_workflow_node_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), BackendError> {
        let is_unpublished = self
            .inner
            .unpublished_workflow_sessions
            .read()
            .map(|sessions| sessions.contains(session_id))
            .map_err(|_poisoned| runtime_unavailable())?;
        if !is_unpublished {
            return Ok(());
        }

        self.delete_session(session_id.as_ref()).await?;
        self.unpublished_workflow_sessions_write()?
            .remove(session_id);
        Ok(())
    }

    /// Moves one existing conversation onto a different agent CLI.
    ///
    /// The binding comes from the warm pool, where the chosen CLI has been
    /// sitting since the picker showed its models. Claiming it rather than
    /// handshaking here is what lets the conversation land on the very session
    /// the user configured, and it means the common case costs no round trip at
    /// all. Like attaching, the claim runs before the lifecycle lock so a CLI
    /// that is slow to answer never stalls unrelated sessions.
    ///
    /// Nothing is torn down until the claim succeeds, so a CLI that is
    /// unavailable leaves the conversation exactly where it was. Only the binding
    /// changes: the identifier, the task, and the recorded history all continue.
    pub(crate) async fn switch_agent(
        &self,
        request: SwitchSessionAgentRequest,
    ) -> Result<SwitchSessionAgentResponse, BackendError> {
        let session = self.find_session(&request.session_id)?;
        let target = domain_agent_ref(request.agent_ref)?;
        // Refused before anything is claimed. Warming the CLI a session already
        // runs on would build a second provider session only to replace the
        // current binding with an indistinguishable one.
        if target == session.agent_ref {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::SessionAgentUnchanged(EmptyErrorParams {}),
                "session already runs on this agent CLI",
            ));
        }
        if let HistoryState::Degraded { .. } = session.history_state {
            return Err(history_degraded());
        }
        let cwd = self.workspace_cwd(&session.workspace_id)?;
        // Keyed by Workspace, the same way the picker warmed it: one warm session per
        // chat surface and CLI, shared by every session under that Task rather
        // than one per conversation.
        let reservation = self
            .inner
            .warm
            .claim(
                WarmKey {
                    target: WarmSessionTarget::Workspace {
                        workspace_id: session.workspace_id.to_string(),
                    },
                    agent_ref: target.clone(),
                    owner: WarmOwner::Interactive,
                },
                &cwd,
            )
            .await?;
        let attachment = reservation.attachment();
        let agent_session_id = attachment.agent_session_id.clone();
        let available_commands = attachment.available_commands.clone();
        let config_options = attachment.config_options.clone();
        // Only now is the move certain, so the old binding can be released. Its
        // context is not reusable afterwards: work done on the new agent would be
        // missing from it, and switching back re-injects the transcript instead.
        let previous = session.agent_ref.clone();

        let response = async {
            let _lifecycle = self.inner.lifecycle.lock().await;
            let supervisor = self.inner.connections.for_agent(&target)?;
            let channel =
                supervisor.open_session_channel(&agent_session_id, session.id.as_ref())?;
            let (session, recorder) = self
                .rebind_to_provider(&session.id, &previous, &target, &agent_session_id)
                .await?;
            self.insert_actor(
                session.clone(),
                ActorSetup {
                    cwd,
                    connection: supervisor,
                    channel: Some(channel),
                    recorder,
                    // The new agent knows nothing; the next prompt carries the transcript.
                    handoff_pending: true,
                    title_acquisition: TitleAcquisition::locked(),
                },
            )?;
            Ok::<_, BackendError>(SwitchSessionAgentResponse {
                session: contract_session(session),
                available_commands,
                config_options,
            })
        }
        .await?;

        reservation.commit();
        Ok(response)
    }

    /// Moves one stored session onto a provider binding that already exists.
    ///
    /// Separate from `switch_agent` because every step here can fail *after* the
    /// provider session was claimed, and each of those failures owes it back to
    /// the warm pool. Keeping them in one fallible region gives the caller a
    /// single place to release the claim — dropping its reservation — instead of
    /// a release per `?`.
    async fn rebind_to_provider(
        &self,
        session_id: &SessionId,
        previous: &AgentRef,
        target: &AgentRef,
        agent_session_id: &str,
    ) -> Result<(Session, SessionRecorder), BackendError> {
        if let Some(handle) = self.lookup_actor(session_id)? {
            self.stop_actor(handle).await?;
        }
        self.actors_write()?.remove(session_id);

        let now = self.inner.clock.now_timestamp_millis();
        let repository = SqliteSessionRepository::new(self.inner.pool.clone());
        repository
            .update_session_binding(session_id, target.clone(), agent_session_id, now)
            .map_err(|source| BackendError::internal("failed to rebind agent session", source))?;
        let session = repository
            .update_session_status(session_id, SessionStatus::Running, now)
            .map_err(|source| BackendError::internal("failed to rebind agent session", source))?;
        ora_debug!(
            session_id = %session.id,
            from = %previous,
            to = %target,
            "session agent switched",
        );

        let mut opened = self.open_recorder(&session)?;
        let outcome = match opened.failure.take() {
            Some(reason) => RecordOutcome::JustFailed { reason },
            None => opened.recorder.record_agent_switch(
                previous.clone(),
                target.clone(),
                agent_session_id.to_string(),
            ),
        };
        Ok((self.settle_record(session, outcome), opened.recorder))
    }

    /// Returns a session whose history writes failed to a writable state.
    ///
    /// The gap is recorded before anything else, so what the failure cost stays
    /// visible to everyone who reads the file afterwards — including the agent
    /// that receives this conversation next.
    pub(crate) async fn resume_history(
        &self,
        request: ResumeSessionHistoryRequest,
    ) -> Result<ResumeSessionHistoryResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let HistoryState::Degraded { reason } = session.history_state.clone() else {
            return Ok(ResumeSessionHistoryResponse {
                session: contract_session(session),
            });
        };
        // The live actor still holds a stopped recorder, so it is discarded and
        // rebuilt from the recovered row on the session's next operation.
        if let Some(handle) = self.lookup_actor(&session.id)? {
            self.stop_actor(handle).await?;
        }
        self.actors_write()?.remove(&session.id);

        let mut opened = self.open_recorder(&session)?;
        if let Some(failure) = opened.failure {
            return Err(BackendError::new(
                ErrorClassification::Internal,
                PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
                format!("session history is still unreadable: {failure}"),
            ));
        }
        if let RecordOutcome::JustFailed { reason } = opened.recorder.resume(reason) {
            return Err(BackendError::new(
                ErrorClassification::Internal,
                PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
                format!("session history is still unwritable: {reason}"),
            ));
        }
        let now = self.inner.clock.now_timestamp_millis();
        let session = SqliteSessionRepository::new(self.inner.pool.clone())
            .update_session_history_state(
                &SessionId::new(request.session_id.clone()),
                &HistoryState::Writable,
                now,
            )
            .map_err(|source| BackendError::internal("failed to resume session history", source))?;
        Ok(ResumeSessionHistoryResponse {
            session: contract_session(session),
        })
    }

    /// Opens one session's recorder, resuming its position counter from the file.
    fn open_recorder(&self, session: &Session) -> Result<OpenedRecorder, BackendError> {
        let root = &self.inner.sessions_root;
        let session_id = session.id.as_ref();
        match read_session_history(root, session_id) {
            Ok(history) => {
                if let HistoryIntegrity::Damaged { unreadable_lines } = history.integrity {
                    ora_warn!(
                        session_id = %session.id,
                        unreadable_lines = unreadable_lines.get(),
                        "session history contains unreadable lines",
                    );
                }
                let recorder = SessionRecorder::open(
                    root,
                    session_id,
                    history.next_seq,
                    &session.history_state,
                    LocalHistoryClock,
                )
                .map_err(|source| {
                    BackendError::internal("failed to open session history", source)
                })?;
                Ok(OpenedRecorder {
                    recorder,
                    handoff_pending: binding_needs_handoff(&history),
                    failure: None,
                })
            }
            Err(error) => {
                // Appending without knowing which positions are already used would
                // overwrite them, so an unreadable file stops recording outright.
                ora_warn!(session_id = %session.id, error = %error, "session history is unreadable");
                let failure = error.to_string();
                let recorder = SessionRecorder::open(
                    root,
                    session_id,
                    0,
                    &HistoryState::Degraded {
                        reason: failure.clone(),
                    },
                    LocalHistoryClock,
                )
                .map_err(|source| {
                    BackendError::internal("failed to open session history", source)
                })?;
                Ok(OpenedRecorder {
                    recorder,
                    handoff_pending: false,
                    failure: Some(failure),
                })
            }
        }
    }

    /// Persists the degraded state when a recording attempt just broke the history.
    fn settle_record(&self, session: Session, outcome: RecordOutcome) -> Session {
        let RecordOutcome::JustFailed { reason } = outcome else {
            return session;
        };
        let now = self.inner.clock.now_timestamp_millis();
        let degraded = session.with_history_state(HistoryState::Degraded { reason }, now);
        match SqliteSessionRepository::new(self.inner.pool.clone()).update_session_history_state(
            &degraded.id,
            &degraded.history_state,
            now,
        ) {
            Ok(stored) => stored,
            Err(error) => {
                ora_warn!(error = %error, "failed to persist degraded session history state");
                degraded
            }
        }
    }

    /// Derives the directory a warm session must be created against.
    fn resolve_warm_cwd(&self, target: &WarmSessionTarget) -> Result<PathBuf, BackendError> {
        match target {
            WarmSessionTarget::Workspace { workspace_id } => {
                self.workspace_cwd(&WorkspaceId::new(workspace_id.as_str()))
            }
        }
    }

    /// Resolves the direct Workspace identity returned alongside a warm provider session.
    fn resolve_warm_workspace_id(
        &self,
        target: &WarmSessionTarget,
    ) -> Result<WorkspaceId, BackendError> {
        match target {
            WarmSessionTarget::Workspace { workspace_id } => {
                Ok(WorkspaceId::new(workspace_id.as_str()))
            }
        }
    }

    /// Resolves a workspace's execution directory without consulting a Task projection.
    pub(crate) fn workspace_cwd(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<PathBuf, BackendError> {
        resolve_workspace_cwd(
            &self.inner.pool,
            workspace_id,
            &self.inner.relative_path_base,
        )
    }

    /// Loads one session conversation, restoring or following its provider turn as needed.
    pub(crate) async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> Result<SessionEventStream<LoadSessionEvent>, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let handle = self.actor_for(session)?;
        let operation_id = self.inner.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let (events_sender, events) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
        let (accepted_sender, accepted) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Load {
                operation_id,
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

    /// Starts one structured ACP prompt stream after validating the public payload limit.
    pub(crate) async fn prompt_session(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        let prompt = request.prompt;
        let record_prompt = request.record_prompt;
        if prompt.is_empty()
            || prompt.iter().all(|content| {
                matches!(content, ContentBlock::Text(text) if text.text.trim().is_empty())
            })
        {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PromptEmpty(EmptyErrorParams {}),
                "prompt must contain text or media",
            ));
        }
        let prompt_bytes = serde_json::to_vec(&prompt)
            .map_err(|_| runtime_internal("prompt_encoding_failed", "failed to encode prompt"))?
            .len();
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PromptTooLarge(EmptyErrorParams {}),
                "prompt exceeds 16 MiB",
            ));
        }
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        if session.status != SessionStatus::Running {
            return Err(session_stopped());
        }
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

    /// Routes one opaque permission response to the actor that registered the request.
    pub(crate) async fn respond_to_permission(
        &self,
        request: RespondToPermissionRequest,
    ) -> Result<RespondToPermissionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let handle = self.actor_for(session)?;
        let (response_sender, response) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::RespondToPermission {
                request,
                response: response_sender,
            })
            .map_err(runtime_unavailable_with)?;
        response.await.map_err(runtime_unavailable_with)?
    }

    /// Cancels the active prompt without unloading the reusable session actor.
    pub(crate) fn cancel_session_prompt(
        &self,
        request: CancelSessionPromptRequest,
    ) -> Result<CancelSessionPromptResponse, BackendError> {
        let session = self.find_session(&request.session_id)?;
        if let Some(handle) = self.lookup_actor(&session.id)? {
            handle
                .commands
                .send(RuntimeCommand::CancelActivePrompt)
                .map_err(runtime_unavailable_with)?;
        }
        Ok(CancelSessionPromptResponse {})
    }

    /// Stops one logical session without terminating its shared CLI process.
    pub(crate) async fn stop_session(
        &self,
        request: StopSessionRequest,
    ) -> Result<StopSessionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let Some(handle) = self.lookup_actor(&session.id)? else {
            return Ok(StopSessionResponse {
                session: contract_session(session),
            });
        };
        self.stop_actor(handle).await
    }

    /// Unloads one actor and removes only the Ora-owned session row.
    pub(crate) async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<DeleteSessionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(session_id)?;
        if let Some(handle) = self.lookup_actor(&session.id)? {
            self.stop_actor(handle).await?;
        }
        let deleted = SqliteSessionRepository::new(self.inner.pool.clone())
            .soft_delete_session(&session.id, self.inner.clock.now_timestamp_millis())
            .map_err(|source| BackendError::internal("failed to delete agent session", source))?;
        if !deleted {
            return Err(session_not_found(session_id));
        }
        self.actors_write()?.remove(&session.id);
        crate::session_history::remove_session_histories(
            &self.inner.sessions_root,
            [session.id.clone()],
        );
        Ok(DeleteSessionResponse {
            session_id: session.id.to_string(),
        })
    }

    /// Waits for an actor to unload its provider session and persist the stopped state.
    async fn stop_actor(
        &self,
        handle: RuntimeActorHandle,
    ) -> Result<StopSessionResponse, BackendError> {
        let (response_sender, response) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Stop {
                response: response_sender,
            })
            .map_err(runtime_unavailable_with)?;
        response.await.map_err(runtime_unavailable_with)?
    }

    /// Loads one non-deleted Ora session from durable storage.
    fn find_session(&self, session_id: &str) -> Result<Session, BackendError> {
        SqliteSessionRepository::new(self.inner.pool.clone())
            .find_session(&SessionId::new(session_id))
            .map_err(|source| BackendError::internal("failed to load session", source))?
            .ok_or_else(|| session_not_found(session_id))
    }

    /// Returns the live actor or restores one lazily after an application restart.
    fn actor_for(&self, session: Session) -> Result<RuntimeActorHandle, BackendError> {
        if let Some(handle) = self.lookup_actor(&session.id)? {
            return Ok(handle);
        }
        let cwd = self.workspace_cwd(&session.workspace_id)?;
        let connection = self.inner.connections.for_agent(&session.agent_ref)?;
        let mut opened = self.open_recorder(&session)?;
        let session = match opened.failure.take() {
            Some(reason) => self.settle_record(session, RecordOutcome::JustFailed { reason }),
            None => session,
        };
        let handoff_pending = opened.handoff_pending;
        self.insert_actor(
            session,
            ActorSetup {
                cwd,
                connection,
                channel: None,
                recorder: opened.recorder,
                handoff_pending,
                title_acquisition: TitleAcquisition::disabled(),
            },
        )
    }

    /// Reads the in-memory actor registry without creating a provider-side session.
    fn lookup_actor(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RuntimeActorHandle>, BackendError> {
        self.inner
            .actors
            .read()
            .map(|actors| actors.get(session_id).cloned())
            .map_err(|_poisoned| runtime_unavailable())
    }

    /// Installs exactly one actor for an Ora session under the lifecycle lock.
    fn insert_actor(
        &self,
        session: Session,
        setup: ActorSetup,
    ) -> Result<RuntimeActorHandle, BackendError> {
        let mut actors = self.actors_write()?;
        if let Some(handle) = actors.get(&session.id) {
            return Ok(handle.clone());
        }
        let (commands, receiver) = mpsc::unbounded_channel();
        let handle = RuntimeActorHandle {
            commands: commands.clone(),
        };
        actors.insert(session.id.clone(), handle.clone());
        tokio::spawn(
            RuntimeActor {
                session,
                cwd: setup.cwd,
                repository: SqliteSessionRepository::new(self.inner.pool.clone()),
                clock: self.inner.clock,
                connection: setup.connection,
                channel: setup.channel,
                commands: receiver,
                recorder: setup.recorder,
                sessions_root: self.inner.sessions_root.clone(),
                handoff_pending: setup.handoff_pending,
                scheduler: self.inner.scheduler.clone(),
                app_events: self.inner.app_events.clone(),
                title_acquisition: setup.title_acquisition,
                command_sender: commands.downgrade(),
                #[cfg(test)]
                exit_probe: None,
            }
            .run(),
        );
        Ok(handle)
    }

    /// Converts registry poisoning into the stable runtime-unavailable contract.
    fn actors_write(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, HashMap<SessionId, RuntimeActorHandle>>, BackendError>
    {
        self.inner
            .actors
            .write()
            .map_err(|_poisoned| runtime_unavailable())
    }

    /// Locks the unpublished workflow Session set for one ownership transition.
    fn unpublished_workflow_sessions_write(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, HashSet<SessionId>>, BackendError> {
        self.inner
            .unpublished_workflow_sessions
            .write()
            .map_err(|_poisoned| runtime_unavailable())
    }
}

/// Groups the provider and persistence state needed to start one session actor.
struct ActorSetup {
    cwd: PathBuf,
    connection: ConnectionSupervisor,
    channel: Option<SessionChannel>,
    recorder: SessionRecorder,
    handoff_pending: bool,
    title_acquisition: TitleAcquisition,
}

/// Builds the refusal returned while a session's history cannot be extended.
fn history_degraded() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
        "session history could not be recorded and must be resumed first",
    )
}

/// Restores durable lifecycle truth before the managed connection starts.
fn reconcile_running_sessions(
    pool: &RepositoryPool,
    clock: SystemClock,
) -> Result<(), BackendError> {
    let repository = SqliteSessionRepository::new(pool.clone());
    for session in repository
        .list_sessions()
        .map_err(|source| BackendError::internal("failed to reconcile sessions", source))?
    {
        if session.status == SessionStatus::Running {
            repository
                .update_session_status(
                    &session.id,
                    SessionStatus::Stopped,
                    clock.now_timestamp_millis(),
                )
                .map_err(|source| BackendError::internal("failed to reconcile sessions", source))?;
        }
    }
    Ok(())
}

/// Extracts the latest setup command catalog while preserving other updates for the first prompt.
async fn collect_setup_commands(channel: &mut SessionChannel) -> Vec<AvailableCommand> {
    let mut available_commands = Vec::new();
    loop {
        // ACP sends setup updates before the response, but the shared router runs
        // independently and may need one short scheduling window to deliver them.
        let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(10), channel.events.recv()).await
        else {
            break;
        };
        match event {
            SessionEvent::Update(notification) => {
                if let SessionUpdate::AvailableCommandsUpdate(update) = &notification.update {
                    // Command updates replace the full catalog, so the last setup value wins.
                    available_commands = update.available_commands.clone();
                } else {
                    channel.pending_updates.push_back(notification);
                }
            }
            SessionEvent::Permission(permission) => {
                let _ = channel
                    .connection
                    .client
                    .respond(
                        &permission.request_id,
                        &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                    )
                    .await;
            }
            SessionEvent::Response(_) => {}
        }
    }
    available_commands
}
