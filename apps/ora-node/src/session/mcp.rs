//! Explicit MCP input, with an opt-in memory owner for ordered lifecycle updates.
pub(crate) mod control;
pub(in crate::session) mod host;
pub(crate) mod probes;
mod replacement;
mod resources;
use super::PluginCatalog;
use ora_agent_runtime::{
    AgentSessionMcpCapabilities, ErrorClassification, InstalledMcpCandidate,
    McpConfigurationEligibility, RuntimeError, SessionMcpCatalog, SessionMcpConfigurationSource,
    SessionMcpError, SessionMcpRevision, SessionMcpSnapshot, SessionMcpTransportKind, SessionSetup,
    resolve_session_mcp, resolve_session_mcp_revision,
};
use ora_contracts::{PublicError, SessionMcpSetupFailedParams};
use ora_domain::{AgentRef, PluginId, SessionId, SessionMcpSelection};
use ora_logging::ora_warn;
use ora_node_protocol::{
    ExecutionId, PluginId as WirePluginId, SessionMcpBinding, SessionMcpTransport,
};
use ora_plugin_config::{CompiledConfigurationFile, SettingValue, compile_configuration_file};
pub(super) use replacement::configure;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// The complete Node terminal vocabulary for explicit Session MCP setup failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NodeMcpError {
    Plugin,
    Setting,
    HttpCapability,
    LoadSession,
}

impl NodeMcpError {
    /// Stable code carried by the terminal result and structured setup failure.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Plugin => "mcp_plugin_missing",
            Self::Setting => "mcp_setting_missing",
            Self::HttpCapability => "mcp_http_capability_missing",
            Self::LoadSession => "mcp_load_session_missing",
        }
    }
}

/// A secret-free failure retained through the runtime source chain for terminal mapping.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{code}", code = .code.as_str())]
pub(super) struct NodeMcpFailure {
    code: NodeMcpError,
    plugin_id: Option<String>,
    setting_id: Option<String>,
    transport: Option<SessionMcpTransportKind>,
}

impl NodeMcpFailure {
    /// Converts to the closed wire vocabulary before publishing any observation.
    fn observation(&self) -> ora_node_protocol::SessionMcpFailure {
        use ora_node_protocol::SessionMcpErrorCode as Code;
        ora_node_protocol::SessionMcpFailure {
            code: match self.code {
                NodeMcpError::Plugin => Code::McpPluginMissing,
                NodeMcpError::Setting => Code::McpSettingMissing,
                NodeMcpError::HttpCapability => Code::McpHttpCapabilityMissing,
                NodeMcpError::LoadSession => Code::McpLoadSessionMissing,
            },
            plugin_id: self.plugin_id.clone().map(WirePluginId::new),
            setting_id: self.setting_id.clone(),
            transport: self.transport.map(|transport| match transport {
                SessionMcpTransportKind::Stdio => SessionMcpTransport::Stdio,
                SessionMcpTransportKind::Http => SessionMcpTransport::Http,
            }),
        }
    }
    /// Names the selected package without retaining a compiler, filesystem or manifest error.
    pub(super) fn plugin(plugin_id: String) -> Self {
        Self {
            code: NodeMcpError::Plugin,
            plugin_id: Some(plugin_id),
            setting_id: None,
            transport: None,
        }
    }

    /// Names only the declaration field whose override or default could not be bound.
    fn setting(plugin_id: String, setting_id: String, transport: SessionMcpTransportKind) -> Self {
        Self {
            code: NodeMcpError::Setting,
            plugin_id: Some(plugin_id),
            setting_id: Some(setting_id),
            transport: Some(transport),
        }
    }

    /// Narrows the shared resolver's rich error to Node's four public terminal codes.
    fn from_resolver(error: &SessionMcpError) -> Self {
        let code = match error {
            SessionMcpError::LoadCapabilityMissing => NodeMcpError::LoadSession,
            SessionMcpError::HttpCapabilityMissing { .. } => NodeMcpError::HttpCapability,
            SessionMcpError::SettingMissing { .. }
            | SessionMcpError::IllegalRuntimeText { .. }
            | SessionMcpError::ConfigurationIncomplete { .. } => NodeMcpError::Setting,
            SessionMcpError::CommandNotInPackage { .. }
            | SessionMcpError::SelectedPluginUnavailable { .. }
            | SessionMcpError::WorkspaceCwdUnresolved { .. }
            | SessionMcpError::RevisionChanged { .. }
            | SessionMcpError::ConfigurationUnavailable { .. }
            | SessionMcpError::CatalogUnavailable => NodeMcpError::Plugin,
        };
        Self {
            code,
            plugin_id: error.plugin_id(),
            setting_id: error.setting_id().map(str::to_owned),
            transport: error.transport(),
        }
    }

    /// Exposes safe member metadata through the same structured shape Desktop uses.
    fn public_error(&self) -> PublicError {
        PublicError::SessionMcpSetupFailed(Box::new(SessionMcpSetupFailedParams {
            error_code: self.code.as_str().to_owned(),
            plugin_id: self.plugin_id.clone(),
            setting_id: self.setting_id.clone(),
            transport: self.transport.map(|kind| kind.as_str().to_owned()),
        }))
    }

    /// Keeps the typed Node code in the source chain instead of reparsing a public string later.
    fn into_runtime_error(self) -> RuntimeError {
        RuntimeError::with_source(
            ErrorClassification::Unprocessable,
            self.public_error(),
            "session MCP setup failed",
            self,
        )
    }

    /// Logs identifiers only; setting values and resolved transport payloads never reach this type.
    fn log(&self, execution: &ExecutionId) {
        ora_warn!(
            execution_id = %execution,
            error_code = self.code.as_str(),
            plugin_id = self.plugin_id.as_deref().unwrap_or("<none>"),
            setting_id = self.setting_id.as_deref().unwrap_or("<none>"),
            transport = self.transport.map(SessionMcpTransportKind::as_str).unwrap_or("<none>"),
            "session MCP preflight failed"
        );
    }

    /// Projects the failure into the terminal envelope without carrying any transport payload.
    fn session_end(&self) -> super::end::SessionEnd {
        super::end::SessionEnd::mcp_failed(
            self.code.as_str(),
            super::end::SessionMcpFailure {
                plugin_id: self
                    .plugin_id
                    .clone()
                    .map(|id| Box::new(WirePluginId::new(id))),
                setting_id: self.setting_id.clone().map(String::into_boxed_str),
                transport: self.transport.map(|transport| match transport {
                    SessionMcpTransportKind::Stdio => SessionMcpTransport::Stdio,
                    SessionMcpTransportKind::Http => SessionMcpTransport::Http,
                }),
            },
        )
    }

    /// Ends an unlaunched session after making the safe failure metadata observable.
    pub(super) fn preflight_end(self, execution: &ExecutionId) -> super::end::SessionEnd {
        self.log(execution);
        self.session_end()
    }
}

/// A session-local immutable package and value snapshot. Debug is intentionally unavailable.
#[derive(Clone)]
pub(super) struct NodeSessionMcp {
    members: Arc<Vec<(InstalledMcpCandidate, BTreeMap<String, SettingValue>)>>,
    selection: SessionMcpSelection,
    generation: u64,
    revisions: BTreeMap<String, u64>,
    lifecycle: Option<(control::Control, Arc<Resolver>)>,
    budget: Arc<std::sync::Mutex<(usize, Option<SessionMcpRevision>)>>,
    replacement: Option<Arc<Replacement>>,
    /// Fixed for the execution; lifecycle resolver views never carry them, because the runtime
    /// asks the session's own view for host servers on every setup.
    host: host::HostServers,
}

/// The catalog type stays with the Node composition; the runtime owns only this session resolver.
type Resolver = dyn Fn(&control::Input) -> Result<NodeSessionMcp, NodeMcpFailure> + Send + Sync;
type Replacement = dyn Fn() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<SessionMcpRevision, RuntimeError>> + Send>,
    > + Send
    + Sync;

impl NodeSessionMcp {
    /// Attaches the execution's Controller-provided servers to every ACP setup of this view.
    pub(super) fn with_host_servers(
        mut self,
        servers: Vec<ora_node_protocol::HostMcpServer>,
    ) -> Self {
        self.host = host::HostServers::new(servers);
        self
    }

    /// Completes once an Agent connection of this execution lacked the HTTP MCP its host
    /// servers need, whether at start, on a refresh or after a replacement.
    pub(super) fn host_refused(&self) -> Arc<tokio::sync::Notify> {
        self.host.refused()
    }

    /// The first process is durably registered against startup package versions before it spawns.
    pub(super) fn with_initial_packages(self, bindings: &[SessionMcpBinding]) -> Self {
        let members = bindings
            .iter()
            .filter_map(|binding| {
                Some(ora_agent_runtime::SessionMcpMemberRevision {
                    plugin_id: PluginId::parse(binding.plugin_id.as_str()).ok()?,
                    package_version: binding.plugin_version.as_str().parse().ok()?,
                    configuration_revision: 0,
                    transport: SessionMcpTransportKind::Stdio,
                })
            })
            .collect();
        *self
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            (0, Some(SessionMcpRevision::new(members)));
        self
    }
    /// Process rotation belongs to the execution owner; runtime policy only requests the boundary.
    pub(super) fn with_replacement(
        mut self,
        replacement: impl Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<SessionMcpRevision, RuntimeError>> + Send>,
        > + Send
        + Sync
        + 'static,
    ) -> Self {
        self.replacement = Some(Arc::new(replacement));
        self
    }
    /// Associates the immutable authorization with its execution-owned mutable target.
    pub(super) fn with_lifecycle(
        mut self,
        control: control::Control,
        resolver: impl Fn(&control::Input) -> Result<Self, NodeMcpFailure> + Send + Sync + 'static,
    ) -> Self {
        self.selection = SessionMcpSelection::Explicit(
            control
                .desired()
                .members
                .iter()
                .filter_map(|member| PluginId::parse(member.plugin_id().as_str()).ok())
                .collect(),
        );
        self.lifecycle = Some((control, Arc::new(resolver)));
        self
    }

    /// Reuses the exact resolver for every complete update; unavailable members block the whole set.
    pub(super) fn prepare_update<P: PluginCatalog>(
        catalog: &P,
        input: &control::Input,
        cwd: &Path,
    ) -> Result<(Self, Vec<P::Lease>), NodeMcpFailure> {
        let mut bindings = Vec::new();
        let mut revisions = BTreeMap::new();
        for member in &input.members {
            match member {
                ora_node_protocol::SessionMcpUpdateMember::Configured {
                    plugin_id,
                    plugin_version,
                    configuration_revision,
                    settings,
                } => {
                    revisions.insert(plugin_id.as_str().to_owned(), *configuration_revision);
                    bindings.push(SessionMcpBinding {
                        plugin_id: plugin_id.clone(),
                        plugin_version: plugin_version.clone(),
                        settings: settings.clone(),
                    });
                }
                ora_node_protocol::SessionMcpUpdateMember::Unavailable { plugin_id, code } => {
                    let mut failure = NodeMcpFailure::plugin(plugin_id.as_str().to_owned());
                    if *code == ora_node_protocol::SessionMcpUnavailable::McpSettingMissing {
                        failure.code = NodeMcpError::Setting;
                    }
                    return Err(failure);
                }
            }
        }
        let (mut setup, leases) = Self::prepare(catalog, bindings, cwd)?;
        setup.generation = input.generation;
        setup.revisions = revisions;
        Ok((setup, leases))
    }

    /// Revision identity never contains resolved credentials.
    fn revision(&self) -> Result<SessionMcpRevision, NodeMcpFailure> {
        resolve_session_mcp_revision(self, self, &self.selection)
            .map(|revision| {
                SessionMcpRevision::for_generation(self.generation, revision.members().to_vec())
            })
            .map_err(|error| NodeMcpFailure::from_resolver(&error))
    }

    /// Rebinds the shared resolver's snapshot to the host generation that supplied its values.
    fn snapshot(
        &self,
        cwd: &Path,
        capabilities: AgentSessionMcpCapabilities,
    ) -> Result<SessionMcpSnapshot, NodeMcpFailure> {
        let snapshot = resolve_session_mcp(self, self, cwd, capabilities, &self.selection)
            .map_err(|error| NodeMcpFailure::from_resolver(&error))?;
        let revision = SessionMcpRevision::for_generation(
            self.generation,
            snapshot.revision().members().to_vec(),
        );
        Ok(SessionMcpSnapshot::new(snapshot.into_servers(), revision))
    }
    /// Pins exact packages before reading declarations; leases remain owned by the driver until
    /// the agent has stopped. A preliminary resolve finds package/setting errors before launch.
    pub(super) fn prepare<P: PluginCatalog>(
        catalog: &P,
        bindings: Vec<SessionMcpBinding>,
        cwd: &Path,
    ) -> Result<(Self, Vec<P::Lease>), NodeMcpFailure> {
        let mut members = Vec::new();
        let mut leases = Vec::new();
        for binding in bindings {
            leases.push(catalog.lease_mcp(&binding.plugin_id, &binding.plugin_version));
            let root = catalog
                .installed(&binding.plugin_id, &binding.plugin_version)
                .ok_or_else(|| NodeMcpFailure::plugin(binding.plugin_id.as_str().to_owned()))?;
            let id = PluginId::parse(binding.plugin_id.as_str())
                .map_err(|_| NodeMcpFailure::plugin(binding.plugin_id.as_str().to_owned()))?;
            let version = binding
                .plugin_version
                .as_str()
                .parse()
                .map_err(|_| NodeMcpFailure::plugin(binding.plugin_id.as_str().to_owned()))?;
            // The production catalog validates this too. Recheck here so injected catalogs cannot
            // bypass package kind, exact-version identity or executable containment.
            ora_plugin_manager::inspect_planned_package(&root, &id, &version)
                .map_err(|_| NodeMcpFailure::plugin(binding.plugin_id.as_str().to_owned()))?;
            let bytes = std::fs::read(root.join("assets").join("config.json"))
                .map_err(|_| NodeMcpFailure::plugin(binding.plugin_id.as_str().to_owned()))?;
            let configuration = match compile_configuration_file(&bytes) {
                Ok(CompiledConfigurationFile::Mcp(configuration)) => configuration,
                Ok(
                    CompiledConfigurationFile::Settings(_)
                    | CompiledConfigurationFile::Hook(_)
                    | CompiledConfigurationFile::Agent(_),
                )
                | Err(_) => {
                    return Err(NodeMcpFailure::plugin(
                        binding.plugin_id.as_str().to_owned(),
                    ));
                }
            };
            let transport = match configuration.transport {
                ora_plugin_config::McpTransport::Stdio(_) => SessionMcpTransportKind::Stdio,
                ora_plugin_config::McpTransport::Http(_) => SessionMcpTransportKind::Http,
            };
            let values = ora_plugin_config::resolve_mcp_settings(&configuration, &binding.settings)
                .map_err(|error| {
                    let setting_id = match error {
                        ora_plugin_config::ResolveMcpBindingError::MissingSetting {
                            setting_id,
                        }
                        | ora_plugin_config::ResolveMcpBindingError::IllegalRuntimeText {
                            setting_id,
                        } => setting_id,
                    };
                    NodeMcpFailure::setting(
                        binding.plugin_id.as_str().to_owned(),
                        setting_id,
                        transport,
                    )
                })?;
            members.push((
                InstalledMcpCandidate {
                    plugin_id: id,
                    version,
                    package_root: root,
                    configuration,
                },
                values,
            ));
        }
        let selection = SessionMcpSelection::Explicit(
            members
                .iter()
                .map(|(candidate, _)| candidate.plugin_id.clone())
                .collect(),
        );
        let setup = Self {
            members: Arc::new(members),
            selection,
            generation: 0,
            revisions: BTreeMap::new(),
            lifecycle: None,
            budget: Arc::new(std::sync::Mutex::new((0, None))),
            replacement: None,
            host: host::HostServers::default(),
        };
        resolve_session_mcp(
            &setup,
            &setup,
            cwd,
            AgentSessionMcpCapabilities::new(/*load_session*/ true, /*http*/ true),
            &setup.selection,
        )
        .map_err(|error| NodeMcpFailure::from_resolver(&error))?;
        Ok((setup, leases))
    }

    /// Makes package bytes readable to a separate workload without opening the Node home.
    pub(super) fn in_placement(
        mut self,
        placement: &super::workload::SessionPlacement,
    ) -> Result<Self, NodeMcpFailure> {
        for (candidate, _) in Arc::make_mut(&mut self.members) {
            candidate.package_root = placement
                .mcp_package(&candidate.plugin_id, &candidate.package_root)
                .map_err(|_| NodeMcpFailure::plugin(candidate.plugin_id.canonical()))?;
        }
        Ok(self)
    }
}

impl SessionMcpCatalog for NodeSessionMcp {
    /// Only the frozen bindings are visible, including when no plugins were selected.
    fn installed_mcps(&self) -> Result<Vec<InstalledMcpCandidate>, SessionMcpError> {
        Ok(self
            .members
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect())
    }
}

impl SessionMcpConfigurationSource for NodeSessionMcp {
    /// Revision zero denotes this immutable session input, never a digest of setting values.
    fn eligibility(
        &self,
        candidate: &InstalledMcpCandidate,
    ) -> Result<McpConfigurationEligibility, SessionMcpError> {
        let values = self
            .members
            .iter()
            .find(|(member, _)| member.plugin_id == candidate.plugin_id)
            .ok_or(SessionMcpError::CatalogUnavailable)?
            .1
            .clone();
        Ok(McpConfigurationEligibility::Complete {
            revision: self
                .revisions
                .get(&candidate.plugin_id.canonical())
                .copied()
                .unwrap_or(0),
            values,
        })
    }
}

impl SessionSetup for NodeSessionMcp {
    fn mcp_replacement_required(&self, revision: &SessionMcpRevision) -> bool {
        if self.lifecycle.is_none() {
            return false;
        }
        let budget = self
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        budget.0 >= 9
            || budget.1.as_ref().is_some_and(|previous| {
                previous
                    .members()
                    .iter()
                    .map(|member| (&member.plugin_id, &member.package_version))
                    .ne(revision
                        .members()
                        .iter()
                        .map(|member| (&member.plugin_id, &member.package_version)))
            })
    }

    async fn replace_mcp_process(&self) -> Result<(), RuntimeError> {
        let revision = match &self.replacement {
            Some(replacement) => Some(replacement().await?),
            None => None,
        };
        *self
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (0, revision);
        Ok(())
    }

    fn mcp_request_started(&self, revision: &SessionMcpRevision) {
        if self.lifecycle.is_some() {
            let mut budget = self
                .budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            budget.0 += 1;
            budget.1 = Some(revision.clone());
        }
    }
    fn preserve_provider_identity(&self) -> bool {
        self.lifecycle.is_some()
    }

    fn mcp_loaded(&self, revision: &SessionMcpRevision) {
        if let Some((control, _)) = &self.lifecycle {
            control.loaded(revision.generation());
        }
    }

    fn mcp_detached(&self) {
        if let Some((control, _)) = &self.lifecycle {
            control.detached();
        }
    }

    fn mcp_failed(&self, error: &RuntimeError) {
        // The host gate ends the execution; it is not a plugin MCP failure to publish.
        if host::is_capability_missing(error) {
            return;
        }
        if let Some((control, _)) = &self.lifecycle {
            let failure = std::error::Error::source(error)
                .and_then(|source| source.downcast_ref::<NodeMcpFailure>())
                .map(NodeMcpFailure::observation)
                .unwrap_or(ora_node_protocol::SessionMcpFailure {
                    code: match error.public_error() {
                        PublicError::AgentTimedOut(_) => {
                            ora_node_protocol::SessionMcpErrorCode::McpRefreshTimeout
                        }
                        PublicError::SessionLoadUnsupported(_) => {
                            ora_node_protocol::SessionMcpErrorCode::McpLoadSessionMissing
                        }
                        _ => ora_node_protocol::SessionMcpErrorCode::McpRefreshFailed,
                    },
                    plugin_id: None,
                    setting_id: None,
                    transport: None,
                });
            control.failed(failure);
        }
    }
    /// Runtime views cannot widen the frozen authorization or enable automatic discovery.
    fn with_selection(&self, _selection: SessionMcpSelection) -> Self {
        self.clone()
    }
    /// Node sessions persist only the explicit plugin identities in their in-memory session row.
    fn selection(&self) -> &SessionMcpSelection {
        &self.selection
    }
    /// Appended after the plugin servers on every `session/new` and `session/load`, including
    /// refresh loads and the setups of a replacement Agent.
    fn host_mcp_servers(&self) -> Vec<agent_client_protocol_schema::v1::McpServer> {
        self.host.servers()
    }
    /// The input cannot change during the lifetime of this execution.
    fn desired_mcp_revision(&self) -> Result<SessionMcpRevision, RuntimeError> {
        if let Some((control, resolver)) = &self.lifecycle {
            let input = control.desired();
            return resolver(&input)
                .and_then(|setup| {
                    control.resolved(input.generation);
                    setup.revision()
                })
                .map_err(|failure| {
                    control.blocked(input.generation, failure.observation());
                    failure.into_runtime_error()
                });
        }
        self.revision().map_err(NodeMcpFailure::into_runtime_error)
    }
    /// Capabilities came from initialize; failure prevents the first session/new request. The
    /// plugin checks come first, so a plugin MCP failure keeps its own code and identity, and the
    /// host gate runs only once the plugin part would have been sent.
    fn resolve_mcp(
        &self,
        cwd: &Path,
        capabilities: AgentSessionMcpCapabilities,
    ) -> Result<SessionMcpSnapshot, RuntimeError> {
        let snapshot = if let Some((control, resolver)) = &self.lifecycle {
            let input = control.begin();
            resolver(&input)
                .and_then(|setup| {
                    control.resolved(input.generation);
                    setup.snapshot(cwd, capabilities)
                })
                .map_err(|failure| {
                    control.blocked(input.generation, failure.observation());
                    failure.into_runtime_error()
                })?
        } else {
            self.snapshot(cwd, capabilities)
                .map_err(NodeMcpFailure::into_runtime_error)?
        };
        self.host.check(capabilities)?;
        Ok(snapshot)
    }
    /// Node health probing is deliberately outside this delivery contract.
    fn observe_mcp_health(&self, _session_id: &SessionId, _cwd: &Path) {}
    /// Agent references retain the runtime's existing canonical plugin identity.
    fn plugin_id_for_agent(&self, agent_ref: &AgentRef) -> Option<PluginId> {
        PluginId::parse(agent_ref.as_str()).ok()
    }
}

/// Logs the same safe identifiers when capability negotiation fails after ACP initialize.
pub(super) fn log_start_failure(error: &RuntimeError, execution: &ExecutionId) {
    if let Some(failure) =
        std::error::Error::source(error).and_then(|source| source.downcast_ref::<NodeMcpFailure>())
    {
        failure.log(execution);
    }
}

/// Ends an ACP capability failure with the same metadata offered to preflight failures.
pub(super) fn start_failure(
    error: &RuntimeError,
    execution: &ExecutionId,
) -> super::end::SessionEnd {
    if let Some(end) = host_start_failure(error, execution) {
        return end;
    }
    log_start_failure(error, execution);
    std::error::Error::source(error)
        .and_then(|source| source.downcast_ref::<NodeMcpFailure>())
        .map_or_else(
            || super::end::SessionEnd::agent_failed("agent_start_failed"),
            NodeMcpFailure::session_end,
        )
}

/// Ends an execution whose host servers met an Agent without HTTP MCP, naming no server.
pub(super) fn host_start_failure(
    error: &RuntimeError,
    execution: &ExecutionId,
) -> Option<super::end::SessionEnd> {
    host::is_capability_missing(error).then(|| {
        ora_warn!(
            execution_id = %execution,
            error_code = host::HOST_MCP_HTTP_CAPABILITY_MISSING,
            "host MCP servers need an Agent with HTTP MCP"
        );
        super::end::SessionEnd::agent_failed(host::HOST_MCP_HTTP_CAPABILITY_MISSING)
    })
}

#[cfg(test)]
mod tests;
