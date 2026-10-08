//! ACP agent runtime shared by every Ora host.
//!
//! The runtime owns ACP session semantics: connection supervision, routing and backpressure, the
//! serialized session actor, prompt inactivity retry, recording, switching, and handoff. Hosts
//! supply storage, plugin processes, session setup, event delivery, and Workspace directories
//! through the traits in [`host`](AgentRuntimeHost).

mod actor;
mod attach;
mod clock;
mod command;
mod connection;
mod error;
mod events;
mod handoff;
mod history;
mod host;
mod limits;
mod load;
mod memory_host;
mod operations;
mod plugin_agent;
mod prompt;
mod prompt_liveness;
mod prompt_retry;
mod record;
mod replaced;
mod replay;
mod restart_circuit;
mod routing;
mod scheduling;
mod session_followers;
mod session_setup;
mod start;
mod stream;
mod support;
mod suspend;
mod title_acquisition;
mod tool_timing;

#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod host_start_tests;
#[cfg(test)]
mod load_tests;
#[cfg(test)]
mod replaced_sessions_tests;
#[cfg(test)]
mod test_host;

pub use error::{ErrorClassification, RuntimeError, SharedError};
pub use host::{
    AgentAttach, AgentPluginAttachment, AgentRuntimeHost, RuntimeEvents, SessionSetup,
    SessionStore, WorkspaceDirectory,
};
pub use memory_host::{MemorySessionStore, MissingSession, NoSessionMcp};
pub use operations::AgentRuntime;
pub use replaced::ReplacedAgentSessions;

/// Effect Consumer calls a host makes against a running agent plugin's control channel.
pub mod plugin_effect {
    pub use crate::plugin_agent::{AgentEffectError, coordinate, reactivate, verify_ready};
}
pub use session_setup::{
    AgentSessionBarrier, AgentSessionBarriers, AgentSessionMcpCapabilities, BarrierGuard,
    BarrierReason, SessionMcpMemberRevision, SessionMcpRevision, SessionMcpSnapshot,
    SessionMcpTransportKind,
};
pub use start::record_session_mcp_boundary;
pub use stream::SessionEventStream;

use attach::RebuiltBinding;
use clock::SystemClock;
use command::RuntimeCommand;
use handoff::HandoffDebt;
use history::{LocalHistoryClock, RecordOutcome, SessionRecorder};
use limits::*;
use prompt::RecordedTurn;
use session_setup::LiveMcpState;
use support::*;
use title_acquisition::TitleAcquisition;

use agent_client_protocol_schema::v1::{
    ContentBlock, MessageId, SessionConfigId, SessionConfigOption, SessionConfigOptionValue,
};
use connection::{ConnectionStatus, ConnectionSupervisor, ConnectionSupervisors};
use ora_contracts::{
    CancelSessionPromptRequest, CancelSessionPromptResponse, DeleteSessionResponse,
    LoadSessionEvent, LoadSessionRequest, PromptSessionEvent, PromptSessionRequest,
    RespondToPermissionRequest, RespondToPermissionResponse, SetSessionConfigRequest,
    SetSessionConfigResponse, StartSessionRequest, StartSessionResponse, StopSessionRequest,
    StopSessionResponse, SwitchSessionAgentRequest, SwitchSessionAgentResponse,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_domain::{
    AgentRef, HistoryState, PromptInactivityPolicy, Session, SessionId, SessionMcpSelection,
    SessionStatus, SessionTitle, WorkspaceId,
};
use ora_history::{binding_needs_handoff, read_session_history};
use ora_logging::{ora_debug, ora_warn};
use ora_scheduler::Scheduler;
use routing::SessionChannel;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};

/// Coordinates one serialized actor per Ora session on its selected supervised CLI connection.
pub struct AgentRuntimeManager<H: AgentRuntimeHost> {
    inner: Arc<ManagerInner<H>>,
}

impl<H: AgentRuntimeHost> Clone for AgentRuntimeManager<H> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct ManagerInner<H: AgentRuntimeHost> {
    store: H::Store,
    directory: H::Directory,
    actors: RwLock<HashMap<SessionId, RuntimeActorHandle>>,
    /// Workflow sessions stay unpublished here until their durable node-run binding exists.
    unpublished_workflow_sessions: RwLock<HashSet<SessionId>>,
    lifecycle: tokio::sync::Mutex<()>,
    next_operation_id: AtomicU64,
    connections: ConnectionSupervisors<H>,
    sessions_root: PathBuf,
    session_mcp: H::Setup,
    barriers: Arc<AgentSessionBarriers>,
    clock: SystemClock,
    scheduler: Scheduler,
    events: H::Events,
}

#[derive(Clone)]
struct RuntimeActorHandle {
    commands: mpsc::UnboundedSender<RuntimeCommand>,
}

struct RuntimeActor<H: AgentRuntimeHost> {
    cleanup_outcomes: HashMap<u64, Result<(), RuntimeError>>,
    session: Session,
    cwd: PathBuf,
    repository: H::Store,
    clock: SystemClock,
    connection: ConnectionSupervisor,
    channel: Option<SessionChannel>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    recorder: SessionRecorder<H::Events>,
    sessions_root: PathBuf,
    /// Opens provider sessions when a prompt needs one this actor does not hold.
    connections: ConnectionSupervisors<H>,
    /// Whether the current provider binding still has to be told the conversation.
    ///
    /// A binding is established eagerly but told lazily, so this is answered from the record when
    /// the actor opens and settled once a prompt carries the transcript across.
    handoff: HandoffDebt,
    /// A provider session built to replace one that could not be restored, not yet in the row.
    rebuilt_binding: Option<RebuiltBinding<H>>,
    /// What the provider this actor is attached to last reported as its configuration.
    ///
    /// Held rather than recorded because it describes the provider serving the conversation right
    /// now, not what was said in it. A load answers with it so that "this session reports options"
    /// means "this session is attached" — the client decides between configuring the live session
    /// and offering the agent's own catalog on exactly that, and inferring it from a stale absence
    /// would silently drop a model chosen for a session that could already have been told.
    reported_config_options: Vec<SessionConfigOption>,
    scheduler: Scheduler,
    events: H::Events,
    title_acquisition: TitleAcquisition,
    command_sender: mpsc::WeakUnboundedSender<RuntimeCommand>,
    session_mcp: H::Setup,
    barriers: Arc<AgentSessionBarriers>,
    live_mcp: LiveMcpState,
    #[cfg(test)]
    exit_probe: Option<oneshot::Sender<()>>,
}

/// Controls whether a newly persisted session is visible before an owning workflow row commits.
#[derive(Clone, Copy)]
enum SessionVisibility {
    Published,
    UnpublishedWorkflow,
}

/// Groups the fixed dependencies the agent runtime is constructed from.
pub struct AgentRuntimeSetup<H: AgentRuntimeHost> {
    /// Owns the processes behind plugin-provided agents and the set of installed packages.
    pub attach: Arc<H::Attach>,
    pub store: H::Store,
    pub directory: H::Directory,
    /// Resolves session MCP with the automatic selection; sessions narrow it to their own.
    pub session_setup: H::Setup,
    pub events: H::Events,
    /// Neutral directory agent plugins start in before any session names a Workspace.
    pub home_directory: PathBuf,
    /// Root of Ora's session history records.
    pub sessions_root: PathBuf,
    pub scheduler: Scheduler,
}

impl<H: AgentRuntimeHost> AgentRuntimeManager<H> {
    /// Builds the manager, reconciles stale rows, and immediately starts the shared supervisor.
    pub fn new(setup: AgentRuntimeSetup<H>) -> Result<Self, RuntimeError> {
        let AgentRuntimeSetup {
            attach,
            store,
            directory,
            session_setup: session_mcp,
            events,
            home_directory,
            sessions_root,
            scheduler,
        } = setup;
        let clock = SystemClock;
        reconcile_running_sessions(&store, clock)?;
        let barriers = Arc::new(AgentSessionBarriers::new());
        let connections =
            ConnectionSupervisors::start(attach, store.clone(), home_directory, clock);
        Ok(Self {
            inner: Arc::new(ManagerInner {
                store,
                directory,
                actors: RwLock::new(HashMap::new()),
                unpublished_workflow_sessions: RwLock::new(HashSet::new()),
                lifecycle: tokio::sync::Mutex::new(()),
                next_operation_id: AtomicU64::new(1),
                connections,
                sessions_root,
                session_mcp,
                barriers,
                clock,
                scheduler,
                events,
            }),
        })
    }

    /// Creates, configures, persists, and binds one session on first use.
    pub async fn start_session(
        &self,
        request: StartSessionRequest,
    ) -> Result<StartSessionResponse, RuntimeError> {
        self.start_session_with_visibility(
            SessionId::new(uuid::Uuid::new_v4().to_string()),
            request,
            SessionVisibility::Published,
            self.inner.session_mcp.clone(),
        )
        .await
    }

    /// Starts a workflow-owned session while keeping it out of ordinary list snapshots.
    pub async fn start_workflow_node_session(
        &self,
        request: StartSessionRequest,
        selection: SessionMcpSelection,
    ) -> Result<StartSessionResponse, RuntimeError> {
        self.start_session_with_visibility(
            SessionId::new(uuid::Uuid::new_v4().to_string()),
            request,
            SessionVisibility::UnpublishedWorkflow,
            self.inner.session_mcp.with_selection(selection),
        )
        .await
    }

    /// Wakes every Live Session so it re-reads the current Desired MCP revision.
    ///
    /// The notification is level-triggered and secret-free. Stopped Sessions ignore it; idle
    /// Sessions refresh immediately; busy Sessions mark refresh as owed work.
    pub fn notify_mcp_desired_changed(&self) {
        let Ok(actors) = self.inner.actors.read() else {
            return;
        };
        for handle in actors.values() {
            let _ = handle.commands.send(RuntimeCommand::McpDesiredMaybeChanged);
        }
    }

    /// Reconciles supervised agent connections with the currently installed plugin set.
    ///
    /// Every plugin operation that changes which packages exist calls this, so installs and
    /// uninstalls are reflected in the agent picker and in session routing without a restart.
    pub fn sync_plugin_agents(&self) {
        self.inner.connections.sync_plugin_agents();
    }

    /// Reports the models one agent advertises before any session exists.
    ///
    /// Discovery is delegated to the plugin on demand and is never cached by Ora.
    pub(crate) async fn agent_models(
        &self,
        request: ora_contracts::ListAgentModelsRequest,
    ) -> Result<ora_contracts::ListAgentModelsResponse, RuntimeError> {
        let cwd = self.workspace_cwd(&WorkspaceId::new(request.workspace_id))?;
        let supervisor = self
            .inner
            .connections
            .for_agent(&domain_agent_ref(request.agent_ref)?)?;
        let connection = supervisor.current()?;
        let models = plugin_agent::list_models(&connection.runtime, &cwd)
            .await
            .map_err(agent_model_discovery_failed)?;
        Ok(ora_contracts::ListAgentModelsResponse {
            models: models
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

    /// Applies one configuration option to a persisted session.
    pub async fn set_session_config(
        &self,
        request: SetSessionConfigRequest,
    ) -> Result<SetSessionConfigResponse, RuntimeError> {
        let config_id = SessionConfigId::new(request.config_id);
        let value = SessionConfigOptionValue::value_id(request.value);
        let session = self.find_session(&request.session_id)?;
        // The live actor is asked which provider session to address rather than reading the row,
        // because a session it rebuilt is not in the row until the transcript reaches it — writing
        // configuration to the row's identity would reach the provider being replaced. A session
        // with no actor holds no rebuilt binding, so its row is authoritative.
        let agent_session_id = match self.lookup_actor(&session.id)? {
            Some(handle) => {
                let (response, claimed) = oneshot::channel();
                handle
                    .commands
                    .send(RuntimeCommand::ClaimDirectProviderCall { response })
                    .map_err(|_error| runtime_unavailable())?;
                claimed.await.map_err(|_error| runtime_unavailable())?
            }
            None => session.agent_session_id.clone(),
        };
        // The provider request remains direct because it is independent of the actor's
        // serialized prompt/load stream; only the title-polling attempt needs standing down.
        let config_options = start::request_config_option(
            &self.inner.connections,
            &session.agent_ref,
            &agent_session_id,
            &config_id,
            &value,
        )
        .await?;
        Ok(SetSessionConfigResponse { config_options })
    }

    /// Locks first-title acquisition so a later agent title cannot overwrite a user rename.
    ///
    /// Missing actors are a no-op: restored sessions already start with acquisition disabled.
    pub async fn adopt_user_title(
        &self,
        session_id: &str,
        title: SessionTitle,
    ) -> Result<(), RuntimeError> {
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

    /// Captures workflow Sessions whose durable node-run binding is not visible yet.
    pub fn unpublished_workflow_session_ids(&self) -> Result<HashSet<String>, RuntimeError> {
        self.inner
            .unpublished_workflow_sessions
            .read()
            .map(|sessions| sessions.iter().map(ToString::to_string).collect())
            .map_err(|_poisoned| runtime_unavailable())
    }

    /// Publishes a workflow Session only after its node-run binding has committed.
    pub fn publish_workflow_node_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), RuntimeError> {
        self.unpublished_workflow_sessions_write()?
            .remove(session_id);
        Ok(())
    }

    /// Removes a workflow Session whose setup failed before any node-run binding was published.
    pub async fn discard_unpublished_workflow_node_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), RuntimeError> {
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
    /// The incoming provider session is fully created before the lifecycle lock is taken.
    /// Nothing is torn down until that handshake succeeds, so a CLI that is
    /// unavailable leaves the conversation exactly where it was. Only the binding
    /// changes: the identifier, the task, and the recorded history all continue.
    pub async fn switch_agent(
        &self,
        request: SwitchSessionAgentRequest,
    ) -> Result<SwitchSessionAgentResponse, RuntimeError> {
        let session = self.find_session(&request.session_id)?;
        let session_mcp = self
            .inner
            .session_mcp
            .with_selection(session.mcp_selection.clone());
        let target = domain_agent_ref(request.agent_ref)?;
        if target == session.agent_ref {
            return Err(RuntimeError::new(
                ErrorClassification::InvalidRequest,
                PublicError::SessionAgentUnchanged(EmptyErrorParams {}),
                "session already runs on this agent CLI",
            ));
        }
        if let HistoryState::Degraded { .. } = session.history_state {
            return Err(history_degraded());
        }
        let cwd = self.workspace_cwd(&session.workspace_id)?;
        let start::PendingProviderSession {
            release,
            agent_session_id,
            channel,
            available_commands,
            config_options,
            mcp_revision,
            ..
        } = self
            .create_provider_session(
                &session.id,
                &target,
                &cwd,
                request.model.as_deref(),
                &session_mcp,
            )
            .await?;
        // Only now is the move certain, so the old binding can be released. Its
        // context is not reusable afterwards: work done on the new agent would be
        // missing from it, and switching back re-injects the transcript instead.
        let previous = session.agent_ref.clone();

        let response = async {
            let _lifecycle = self.inner.lifecycle.lock().await;
            let supervisor = self.inner.connections.for_agent(&target)?;
            let (session, recorder) = self
                .rebind_to_provider(&session.id, &previous, &target, &agent_session_id)
                .await?;
            self.insert_actor(
                session.clone(),
                ActorSetup {
                    session_mcp,
                    cwd,
                    connection: supervisor,
                    channel: Some(channel),
                    recorder,
                    // The new agent knows nothing; the next prompt carries the transcript.
                    handoff: HandoffDebt::Recorded,
                    title_acquisition: TitleAcquisition::locked(),
                    live_mcp: LiveMcpState::Active(mcp_revision),
                    config_options: config_options.clone(),
                },
            )?;
            Ok::<_, RuntimeError>(SwitchSessionAgentResponse {
                session: contract_session(session),
                available_commands,
                config_options,
            })
        }
        .await?;

        release.commit();
        Ok(response)
    }

    /// Moves one stored session onto a freshly-created provider binding.
    async fn rebind_to_provider(
        &self,
        session_id: &SessionId,
        previous: &AgentRef,
        target: &AgentRef,
        agent_session_id: &str,
    ) -> Result<(Session, SessionRecorder<H::Events>), RuntimeError> {
        if let Some(handle) = self.lookup_actor(session_id)? {
            self.stop_actor(handle).await?;
        }
        self.actors_write()?.remove(session_id);

        let now = self.inner.clock.now_timestamp_millis();
        let repository = &self.inner.store;
        repository
            .update_session_binding(session_id, target.clone(), agent_session_id, now)
            .map_err(|source| RuntimeError::internal("failed to rebind agent session", source))?;
        let session = repository
            .update_session_status(session_id, SessionStatus::Running, now)
            .map_err(|source| RuntimeError::internal("failed to rebind agent session", source))?;
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

    /// Routes one opaque permission response to the actor that registered the request.
    pub async fn respond_to_permission(
        &self,
        request: RespondToPermissionRequest,
    ) -> Result<RespondToPermissionResponse, RuntimeError> {
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
    pub fn cancel_session_prompt(
        &self,
        request: CancelSessionPromptRequest,
    ) -> Result<CancelSessionPromptResponse, RuntimeError> {
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
    pub async fn stop_session(
        &self,
        request: StopSessionRequest,
    ) -> Result<StopSessionResponse, RuntimeError> {
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
    pub async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<DeleteSessionResponse, RuntimeError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(session_id)?;
        if let Some(handle) = self.lookup_actor(&session.id)? {
            self.stop_actor(handle).await?;
        }
        let deleted = self
            .inner
            .store
            .soft_delete_session(&session.id, self.inner.clock.now_timestamp_millis())
            .map_err(|source| RuntimeError::internal("failed to delete agent session", source))?;
        if !deleted {
            return Err(session_not_found(session_id));
        }
        self.actors_write()?.remove(&session.id);
        // Best effort, like every history removal: the row is already gone, and a file left
        // behind is unreachable while failing here would leave the user unable to delete.
        if let Err(error) =
            ora_history::remove_session_history(&self.inner.sessions_root, session.id.as_ref())
        {
            ora_warn!(
                session_id = %session.id,
                error = %error,
                "failed to remove session history file",
            );
        }
        Ok(DeleteSessionResponse {
            session_id: session.id.to_string(),
        })
    }

    /// Waits for an actor to unload its provider session and persist the stopped state.
    async fn stop_actor(
        &self,
        handle: RuntimeActorHandle,
    ) -> Result<StopSessionResponse, RuntimeError> {
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
    fn find_session(&self, session_id: &str) -> Result<Session, RuntimeError> {
        self.inner
            .store
            .find_session(&SessionId::new(session_id))
            .map_err(|source| RuntimeError::internal("failed to load session", source))?
            .ok_or_else(|| session_not_found(session_id))
    }

    /// Returns the live actor or restores one lazily after an application restart.
    fn actor_for(&self, session: Session) -> Result<RuntimeActorHandle, RuntimeError> {
        if let Some(handle) = self.lookup_actor(&session.id)? {
            return Ok(handle);
        }
        let cwd = self.workspace_cwd(&session.workspace_id)?;
        let connection = self.inner.connections.for_agent(&session.agent_ref)?;
        let session_mcp = self
            .inner
            .session_mcp
            .with_selection(session.mcp_selection.clone());
        let mut opened = self.open_recorder(&session)?;
        let session = match opened.failure.take() {
            Some(reason) => self.settle_record(session, RecordOutcome::JustFailed { reason }),
            None => session,
        };
        let handoff = if opened.handoff_pending {
            HandoffDebt::Recorded
        } else {
            HandoffDebt::Settled
        };
        self.insert_actor(
            session,
            ActorSetup {
                session_mcp,
                cwd,
                connection,
                channel: None,
                recorder: opened.recorder,
                handoff,
                title_acquisition: TitleAcquisition::disabled(),
                live_mcp: LiveMcpState::Inactive,
                config_options: Vec::new(),
            },
        )
    }

    /// Reads the in-memory actor registry without creating a provider-side session.
    fn lookup_actor(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RuntimeActorHandle>, RuntimeError> {
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
        setup: ActorSetup<H>,
    ) -> Result<RuntimeActorHandle, RuntimeError> {
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
                cleanup_outcomes: HashMap::new(),
                session,
                cwd: setup.cwd,
                repository: self.inner.store.clone(),
                clock: self.inner.clock,
                connection: setup.connection,
                channel: setup.channel,
                commands: receiver,
                recorder: setup.recorder,
                sessions_root: self.inner.sessions_root.clone(),
                connections: self.inner.connections.clone(),
                handoff: setup.handoff,
                rebuilt_binding: None,
                reported_config_options: setup.config_options,
                scheduler: self.inner.scheduler.clone(),
                events: self.inner.events.clone(),
                title_acquisition: setup.title_acquisition,
                command_sender: commands.downgrade(),
                session_mcp: setup.session_mcp,
                barriers: self.inner.barriers.clone(),
                live_mcp: setup.live_mcp,
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
    ) -> Result<std::sync::RwLockWriteGuard<'_, HashMap<SessionId, RuntimeActorHandle>>, RuntimeError>
    {
        self.inner
            .actors
            .write()
            .map_err(|_poisoned| runtime_unavailable())
    }

    /// Locks the unpublished workflow Session set for one ownership transition.
    fn unpublished_workflow_sessions_write(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, HashSet<SessionId>>, RuntimeError> {
        self.inner
            .unpublished_workflow_sessions
            .write()
            .map_err(|_poisoned| runtime_unavailable())
    }
}

/// Groups the provider and persistence state needed to start one session actor.
struct ActorSetup<H: AgentRuntimeHost> {
    session_mcp: H::Setup,
    cwd: PathBuf,
    connection: ConnectionSupervisor,
    channel: Option<SessionChannel>,
    recorder: SessionRecorder<H::Events>,
    handoff: HandoffDebt,
    title_acquisition: TitleAcquisition,
    live_mcp: LiveMcpState,
    /// What the handshake that produced `channel` reported, empty when there is no channel.
    config_options: Vec<SessionConfigOption>,
}

/// Restores durable lifecycle truth before the managed connection starts.
fn reconcile_running_sessions(
    repository: &impl SessionStore,
    clock: SystemClock,
) -> Result<(), RuntimeError> {
    for session in repository
        .list_sessions()
        .map_err(|source| RuntimeError::internal("failed to reconcile sessions", source))?
    {
        if session.status == SessionStatus::Running {
            repository
                .update_session_status(
                    &session.id,
                    SessionStatus::Stopped,
                    clock.now_timestamp_millis(),
                )
                .map_err(|source| RuntimeError::internal("failed to reconcile sessions", source))?;
        }
    }
    Ok(())
}
