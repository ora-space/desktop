//! Creates provider sessions only when a caller is ready to persist and use them.

use super::connection::ConnectionSupervisors;
use super::history::RecordOutcome;
use super::routing::SessionChannel;
use super::support::{agent_timed_out, map_acp_error};
use super::support::{contract_session, domain_agent_ref};
use super::{ActorSetup, HandoffDebt, SessionVisibility, TitleAcquisition};
use super::{AgentRuntimeManager, SESSION_SETUP_TIMEOUT, collect_setup_commands};
use crate::BackendError;
use crate::session_setup::{
    AgentSessionMcpCapabilities, SessionMcpHost, SessionMcpRevision, SessionSetup,
};
use crate::session_setup::{LiveMcpState, SessionMcpSelection};
use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, AvailableCommand, CloseSessionRequest, CloseSessionResponse,
    DeleteSessionRequest, DeleteSessionResponse, NewSessionRequest, NewSessionResponse,
    SessionConfigId, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigOptionValue, SessionConfigSelectOptions, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse,
};
use ora_application::{Clock, SessionIdGenerator, SessionRepository, UuidSessionIdGenerator};
use ora_contracts::{StartSessionRequest, StartSessionResponse};
use ora_db::SqliteSessionRepository;
use ora_domain::{AgentRef, SessionId};
use ora_domain::{AuditFields, Session, SessionStatus, WorkspaceId};
use ora_logging::{ora_debug, ora_info, ora_warn};
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;

const SESSION_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything one authoritative provider handshake produced, plus its rollback guard.
///
/// Destructured by the caller rather than accessed through methods: the route channel has to move
/// into the actor while the guard stays behind covering every later step, and owning both in one
/// value that is never partially valid is what removes the "channel already taken" state entirely.
pub(super) struct PendingProviderSession {
    /// Releases the provider session unless the caller commits, including on cancellation.
    pub(super) release: ProviderSessionRelease,
    pub(super) agent_session_id: String,
    /// The capability captured from the same connection generation as this session.
    pub(super) list_session_supported: bool,
    /// The only route channel opened for this session; the actor takes it over verbatim.
    pub(super) channel: SessionChannel,
    pub(super) available_commands: Vec<AvailableCommand>,
    pub(super) config_options: Vec<SessionConfigOption>,
    /// Secret-free identity of the MCP Snapshot sent with this `session/new`.
    pub(super) mcp_revision: SessionMcpRevision,
}

/// Owns a provider session that no Ora record points at yet.
///
/// Dropping an uncommitted guard schedules best-effort cleanup. That covers ordinary errors and
/// the command future being cancelled after `session/new` already succeeded — the window in which
/// a provider session would otherwise survive with nothing left to close it.
pub(super) struct ProviderSessionRelease {
    connections: ConnectionSupervisors,
    agent_ref: AgentRef,
    generation: u64,
    agent_session_id: String,
    committed: bool,
}

impl ProviderSessionRelease {
    /// Transfers cleanup responsibility to the persisted session actor.
    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ProviderSessionRelease {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let connections = self.connections.clone();
        let agent_ref = self.agent_ref.clone();
        let agent_session_id = self.agent_session_id.clone();
        let generation = self.generation;
        tokio::spawn(async move {
            release_provider_session(&connections, &agent_ref, &agent_session_id, generation).await;
        });
    }
}

impl AgentRuntimeManager {
    /// Runs the only path allowed to create and persist a provider session.
    pub(super) async fn start_session_with_visibility(
        &self,
        request: StartSessionRequest,
        visibility: SessionVisibility,
        session_mcp: SessionMcpHost,
    ) -> Result<StartSessionResponse, BackendError> {
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let agent_ref = domain_agent_ref(request.agent_ref)?;
        let cwd = self.workspace_cwd(&workspace_id)?;
        let session_id = UuidSessionIdGenerator::new().generate_session_id();
        let PendingProviderSession {
            release,
            agent_session_id,
            list_session_supported,
            channel,
            available_commands,
            config_options,
            mcp_revision,
        } = self
            .create_provider_session(
                &session_id,
                &agent_ref,
                &cwd,
                request.model.as_deref(),
                &session_mcp,
            )
            .await?;
        let mut persisted = false;
        let unpublished = matches!(visibility, SessionVisibility::UnpublishedWorkflow);

        if unpublished {
            self.unpublished_workflow_sessions_write()?
                .insert(session_id.clone());
        }
        let result = async {
            let _lifecycle = self.inner.lifecycle.lock().await;
            let supervisor = self.inner.connections.for_agent(&agent_ref)?;
            let now = self.inner.clock.now_timestamp_millis();
            let session = Session::new(
                session_id.clone(),
                workspace_id,
                agent_ref,
                agent_session_id,
                SessionStatus::Running,
                AuditFields::new(now, now, false),
            );
            let mut opened = self.open_recorder(&session)?;
            let outcome = match opened.failure.take() {
                Some(reason) => RecordOutcome::JustFailed { reason },
                None => opened.recorder.record_meta(&session, &cwd),
            };
            SqliteSessionRepository::new(self.inner.pool.clone())
                .create_session(session.clone())
                .map_err(|source| {
                    BackendError::internal("failed to persist agent CLI session", source)
                })?;
            persisted = true;
            let session = self.settle_record(session, outcome);
            let title_acquisition = TitleAcquisition::awaiting_first_prompt(list_session_supported);
            self.insert_actor(
                session.clone(),
                ActorSetup {
                    session_mcp,
                    cwd,
                    connection: supervisor,
                    channel: Some(channel),
                    recorder: opened.recorder,
                    handoff: HandoffDebt::Settled,
                    title_acquisition,
                    live_mcp: LiveMcpState::Active(mcp_revision),
                    config_options: config_options.clone(),
                },
            )?;
            Ok::<_, BackendError>(StartSessionResponse {
                session: contract_session(session),
                available_commands,
                config_options,
            })
        }
        .await;

        match result {
            Ok(response) => {
                release.commit();
                Ok(response)
            }
            Err(error) => {
                if persisted {
                    let _ = SqliteSessionRepository::new(self.inner.pool.clone())
                        .soft_delete_session(&session_id, self.inner.clock.now_timestamp_millis());
                }
                let _ = ora_history::remove_session_history(
                    &self.inner.sessions_root,
                    session_id.as_ref(),
                );
                if unpublished {
                    self.unpublished_workflow_sessions_write()?
                        .remove(&session_id);
                }
                Err(error)
            }
        }
    }

    /// Creates a provider session and keeps it unpublished until the caller persists ownership.
    pub(super) async fn create_provider_session(
        &self,
        ora_session_id: &SessionId,
        agent_ref: &AgentRef,
        cwd: &Path,
        model: Option<&str>,
        session_mcp: &SessionMcpHost,
    ) -> Result<PendingProviderSession, BackendError> {
        create_provider_session(
            &self.inner.connections,
            session_mcp,
            ora_session_id,
            agent_ref,
            cwd,
            model,
        )
        .await
    }
}

/// Performs one authoritative provider handshake and applies a pre-session model intent.
pub(super) async fn create_provider_session(
    connections: &ConnectionSupervisors,
    session_mcp: &SessionMcpHost,
    ora_session_id: &SessionId,
    agent_ref: &AgentRef,
    cwd: &Path,
    model: Option<&str>,
) -> Result<PendingProviderSession, BackendError> {
    let supervisor = connections.for_agent(agent_ref)?;
    let connection = supervisor.current()?;
    let setup = SessionSetup::resolve(
        session_mcp,
        cwd,
        AgentSessionMcpCapabilities::new(
            connection.load_session_supported,
            connection.http_mcp_supported,
        ),
    )
    .map_err(crate::session_setup::SessionMcpError::into_backend)?;
    log_session_mcp_request(
        ora_session_id,
        agent_ref,
        None,
        AGENT_METHOD_NAMES.session_new,
        &session_mcp.selection,
        &setup.mcp,
    );
    let mcp_revision = setup.mcp.revision().clone();
    let _setup_registration = supervisor.begin_session_setup();
    let response = timeout(
        SESSION_SETUP_TIMEOUT,
        connection.client.request::<_, NewSessionResponse>(
            AGENT_METHOD_NAMES.session_new,
            &NewSessionRequest::new(cwd).mcp_servers(setup.mcp.into_servers()),
        ),
    )
    .await
    .map_err(|_| agent_timed_out("agent CLI session creation timed out"))?
    .map_err(map_acp_error)?;
    let agent_session_id = response.session_id.to_string();
    ora_debug!(
        agent = %agent_ref,
        agent_session_id,
        "provider session created for immediate persistence",
    );
    // The guard exists from here on, so every `?` below releases the session it names.
    let release = ProviderSessionRelease {
        connections: connections.clone(),
        agent_ref: agent_ref.clone(),
        generation: connection.generation,
        agent_session_id: agent_session_id.clone(),
        committed: false,
    };
    let mut channel =
        supervisor.open_session_channel(&agent_session_id, ora_session_id.as_ref())?;
    // Collected on the channel the actor inherits, so the non-command setup updates the agent sent
    // alongside them stay queued on it instead of being discarded with a throwaway registration.
    let available_commands = collect_setup_commands(&mut channel).await;
    let mut config_options = response.config_options.unwrap_or_default();
    if let Some(model) = model {
        config_options = apply_model_intent(
            connections,
            agent_ref,
            &agent_session_id,
            model,
            config_options,
        )
        .await;
    }
    Ok(PendingProviderSession {
        release,
        list_session_supported: connection.list_session_supported,
        agent_session_id,
        channel,
        available_commands,
        config_options,
        mcp_revision,
    })
}

/// One selected MCP member rendered for logs without executable configuration or credentials.
struct SessionMcpLogMember {
    plugin_id: String,
    package_version: String,
    configuration_revision: u64,
    transport: &'static str,
}

impl std::fmt::Debug for SessionMcpLogMember {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionMcpLogMember")
            .field("plugin_id", &self.plugin_id)
            .field("package_version", &self.package_version)
            .field("configuration_revision", &self.configuration_revision)
            .field("transport", &self.transport)
            .finish()
    }
}

/// Logs the exact secret-free MCP identity sent at an ACP session boundary.
pub(super) fn log_session_mcp_request(
    ora_session_id: &SessionId,
    agent_ref: &AgentRef,
    agent_session_id: Option<&str>,
    acp_method: &str,
    selection: &SessionMcpSelection,
    snapshot: &crate::session_setup::SessionMcpSnapshot,
) {
    let members = snapshot
        .revision()
        .members()
        .iter()
        .map(|member| SessionMcpLogMember {
            plugin_id: member.plugin_id.canonical(),
            package_version: member.package_version.to_string(),
            configuration_revision: member.configuration_revision,
            transport: member.transport.as_str(),
        })
        .collect::<Vec<_>>();
    ora_info!(
        session_id = %ora_session_id,
        agent = %agent_ref,
        agent_session_id,
        acp_method,
        mcp_selection = selection.mode(),
        mcp_server_count = snapshot.servers().len(),
        mcp_members = ?members,
        "sending ACP session configuration"
    );
}

/// Applies a model only when the session authoritatively offers that value.
pub(super) async fn apply_model_intent(
    connections: &ConnectionSupervisors,
    agent_ref: &AgentRef,
    agent_session_id: &str,
    model: &str,
    config_options: Vec<SessionConfigOption>,
) -> Vec<SessionConfigOption> {
    let Some(config_id) = model_config_id(&config_options, model) else {
        return config_options;
    };
    match request_config_option(
        connections,
        agent_ref,
        agent_session_id,
        &config_id,
        &SessionConfigOptionValue::value_id(model.to_string()),
    )
    .await
    {
        Ok(updated) => updated,
        Err(error) => {
            ora_warn!(
                agent = %agent_ref,
                model,
                error = %error,
                "provider rejected pre-session model intent",
            );
            config_options
        }
    }
}

/// Finds the model selector only from the options reported by this exact handshake.
fn model_config_id(config_options: &[SessionConfigOption], model: &str) -> Option<SessionConfigId> {
    let option = config_options
        .iter()
        .find(|option| matches!(option.category, Some(SessionConfigOptionCategory::Model)))
        .or_else(|| {
            let mut selects = config_options
                .iter()
                .filter(|option| matches!(option.kind, SessionConfigKind::Select(_)));
            let only = selects.next()?;
            selects.next().is_none().then_some(only)
        })?;
    let SessionConfigKind::Select(select) = &option.kind else {
        return None;
    };
    let offered = match &select.options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .any(|option| option.value.0.as_ref() == model),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .any(|option| option.value.0.as_ref() == model),
        _ => false,
    };
    offered.then(|| option.id.clone())
}

/// Sends one persisted-session configuration request and returns the provider's full report.
pub(super) async fn request_config_option(
    connections: &ConnectionSupervisors,
    agent_ref: &AgentRef,
    agent_session_id: &str,
    config_id: &SessionConfigId,
    value: &SessionConfigOptionValue,
) -> Result<Vec<SessionConfigOption>, BackendError> {
    let connection = connections.for_agent(agent_ref)?.current()?;
    let response = timeout(
        SESSION_SETUP_TIMEOUT,
        connection
            .client
            .request::<_, SetSessionConfigOptionResponse>(
                AGENT_METHOD_NAMES.session_set_config_option,
                &SetSessionConfigOptionRequest::new(
                    agent_session_id.to_string(),
                    config_id.clone(),
                    value.clone(),
                ),
            ),
    )
    .await
    .map_err(|_| agent_timed_out("agent configuration timed out"))?
    .map_err(map_acp_error)?;
    Ok(response.config_options)
}

/// Releases a provider session that never became visible to a user.
async fn release_provider_session(
    connections: &ConnectionSupervisors,
    agent_ref: &AgentRef,
    agent_session_id: &str,
    generation: u64,
) {
    let Ok(connection) = connections
        .for_agent(agent_ref)
        .and_then(|supervisor| supervisor.current())
    else {
        return;
    };
    if connection.generation != generation {
        return;
    }
    if connection.delete_session_supported {
        let _ = timeout(
            SESSION_RELEASE_TIMEOUT,
            connection.client.request::<_, DeleteSessionResponse>(
                AGENT_METHOD_NAMES.session_delete,
                &DeleteSessionRequest::new(agent_session_id.to_string()),
            ),
        )
        .await;
    } else if connection.close_session_supported {
        let _ = timeout(
            SESSION_RELEASE_TIMEOUT,
            connection.client.request::<_, CloseSessionResponse>(
                AGENT_METHOD_NAMES.session_close,
                &CloseSessionRequest::new(agent_session_id.to_string()),
            ),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::{log_session_mcp_request, model_config_id};
    use crate::session_setup::{
        SessionMcpMemberRevision, SessionMcpRevision, SessionMcpSelection, SessionMcpSnapshot,
        SessionMcpTransportKind,
    };
    use agent_client_protocol_schema::v1::{
        HttpHeader, McpServer, McpServerHttp, SessionConfigGroupId, SessionConfigId,
        SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
        SessionConfigSelectGroup, SessionConfigSelectOption, SessionConfigSelectOptions,
        SessionConfigValueId,
    };
    use ora_domain::{AgentRef, PluginId, SessionId};
    use ora_logging::with_recorded_trace_logging;
    use pretty_assertions::assert_eq;
    use semver::Version;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};

    fn select_option(value: &str, name: &str) -> SessionConfigSelectOption {
        SessionConfigSelectOption::new(
            SessionConfigValueId::new(value.to_string()),
            name.to_string(),
        )
    }

    fn select(id: &str, options: SessionConfigSelectOptions) -> SessionConfigOption {
        SessionConfigOption::new(
            SessionConfigId::new(id.to_string()),
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                SessionConfigValueId::new("current".to_string()),
                options,
            )),
        )
    }

    /// The intent is applied only against the selector the fresh handshake itself declared.
    ///
    /// A pre-session pick is client-side intent with nothing authoritative behind it, so the
    /// session that was just created is the only thing allowed to say which selector owns models.
    #[test]
    fn resolves_the_declared_model_selector() {
        let options = vec![
            select("thinking", SessionConfigSelectOptions::Ungrouped(vec![])),
            select(
                "model",
                SessionConfigSelectOptions::Ungrouped(vec![
                    select_option("big", "Big"),
                    select_option("small", "Small"),
                ]),
            )
            .category(SessionConfigOptionCategory::Model),
        ];

        assert_eq!(
            model_config_id(&options, "small"),
            Some(SessionConfigId::new("model".to_string())),
        );
    }

    /// Grouped selectors are flattened, because the picker offers one flat list of values.
    #[test]
    fn resolves_a_model_offered_inside_a_group() {
        let options = vec![
            select(
                "model",
                SessionConfigSelectOptions::Grouped(vec![SessionConfigSelectGroup::new(
                    SessionConfigGroupId::new("anthropic".to_string()),
                    "Anthropic",
                    vec![select_option("claude/haiku", "Haiku")],
                )]),
            )
            .category(SessionConfigOptionCategory::Model),
        ];

        assert_eq!(
            model_config_id(&options, "claude/haiku"),
            Some(SessionConfigId::new("model".to_string())),
        );
    }

    /// An agent that categorises nothing still has exactly one selector, which must be the models.
    #[test]
    fn falls_back_to_a_sole_uncategorised_selector() {
        let options = vec![select(
            "model",
            SessionConfigSelectOptions::Ungrouped(vec![select_option("smart", "Smart")]),
        )];

        assert_eq!(
            model_config_id(&options, "smart"),
            Some(SessionConfigId::new("model".to_string())),
        );
    }

    /// Two uncategorised selectors are ambiguous, and guessing would configure the wrong one.
    #[test]
    fn refuses_to_guess_between_two_uncategorised_selectors() {
        let options = vec![
            select(
                "model",
                SessionConfigSelectOptions::Ungrouped(vec![select_option("smart", "Smart")]),
            ),
            select(
                "thinking",
                SessionConfigSelectOptions::Ungrouped(vec![select_option("smart", "Smart")]),
            ),
        ];

        assert_eq!(model_config_id(&options, "smart"), None);
    }

    /// A model the new session does not offer is never requested.
    ///
    /// The intent was recorded against a catalog the plugin answered separately, which can name a
    /// value this session will not accept. Sending it anyway would trade a picker that silently
    /// shows the agent's default for a failed configuration request on the user's first send.
    #[test]
    fn does_not_request_a_value_the_new_session_never_offered() {
        let options = vec![
            select(
                "model",
                SessionConfigSelectOptions::Ungrouped(vec![select_option("big", "Big")]),
            )
            .category(SessionConfigOptionCategory::Model),
        ];

        assert_eq!(model_config_id(&options, "retired-model"), None);
    }

    /// ACP boundary diagnostics identify the selection without serializing server credentials.
    #[test]
    fn session_mcp_log_contains_revision_identity_and_omits_secrets() {
        let plugin_id = PluginId::parse("official/tavily").expect("plugin id");
        let snapshot = SessionMcpSnapshot::new(
            vec![McpServer::Http(
                McpServerHttp::new(plugin_id.canonical(), "https://secret.example.test/mcp")
                    .headers(vec![HttpHeader::new(
                        "Authorization",
                        "Bearer super-secret",
                    )]),
            )],
            SessionMcpRevision::new(vec![SessionMcpMemberRevision {
                plugin_id: plugin_id.clone(),
                package_version: Version::new(1, 2, 3),
                configuration_revision: 7,
                transport: SessionMcpTransportKind::Http,
            }]),
        );
        let selection = SessionMcpSelection::Explicit(BTreeSet::from([plugin_id.canonical()]));
        let recorder = EventTextRecorder::default();

        with_recorded_trace_logging(recorder.layer(), || {
            log_session_mcp_request(
                &SessionId::new("ora-session"),
                &AgentRef::parse("official/opencode").expect("agent ref"),
                Some("provider-session"),
                "session/new",
                &selection,
                &snapshot,
            );
        });

        let recorded = recorder.text();
        assert!(
            recorded.contains("sending ACP session configuration"),
            "{recorded}"
        );
        assert!(recorded.contains("official/tavily"), "{recorded}");
        assert!(recorded.contains("1.2.3"), "{recorded}");
        assert!(recorded.contains("configuration_revision: 7"), "{recorded}");
        assert!(recorded.contains("explicit"), "{recorded}");
        assert!(!recorded.contains("super-secret"));
        assert!(!recorded.contains("secret.example.test"));
        assert!(!recorded.contains("Authorization"));
    }

    /// Captures the rendered fields from one test-scoped logging event.
    #[derive(Clone, Debug, Default)]
    struct EventTextRecorder {
        text: Arc<Mutex<String>>,
    }

    impl EventTextRecorder {
        /// Builds a subscriber layer sharing this recorder's output.
        fn layer(&self) -> EventTextLayer {
            EventTextLayer {
                text: self.text.clone(),
            }
        }

        /// Returns every field rendered by the captured event.
        fn text(&self) -> String {
            self.text.lock().expect("recorded event lock").clone()
        }
    }

    /// Records field names and values without using the production formatter.
    #[derive(Clone, Debug)]
    struct EventTextLayer {
        text: Arc<Mutex<String>>,
    }

    impl<S> Layer<S> for EventTextLayer
    where
        S: tracing::Subscriber,
    {
        /// Appends each event field under the test-scoped TRACE subscriber.
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            event.record(&mut EventTextVisitor {
                text: self.text.clone(),
            });
        }
    }

    /// Renders structured values so the test can assert the complete leak boundary.
    struct EventTextVisitor {
        text: Arc<Mutex<String>>,
    }

    impl Visit for EventTextVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            let mut text = self.text.lock().expect("recorded event lock");
            text.push_str(field.name());
            text.push('=');
            text.push_str(value);
            text.push('\n');
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.record_debug(field, &value);
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            let mut text = self.text.lock().expect("recorded event lock");
            text.push_str(field.name());
            text.push('=');
            text.push_str(&format!("{value:?}"));
            text.push('\n');
        }
    }
}
