//! In-memory Host MCP health: identity-keyed status, single-flight probes, Session observation.
//!
//! Health is orthogonal to delivery. A probe failure never fails session setup, never shrinks the
//! Effective MCP Set, never produces a partial `mcpServers` list, and never writes Setting values
//! into DTOs, events, query responses, or structured logs. Every status lives in this process only;
//! a restart always starts from `Unknown(not_probed)`, and nothing here is persisted or rechecked
//! on a timer.

mod probe;

#[cfg(test)]
mod tests;

pub(crate) use super::member::EligibleMcpMember;
use super::member::{effective_members, find_eligible_member};
use super::{
    SessionMcpCatalog, SessionMcpConfigurationSource, SessionMcpError, SessionMcpHost,
    SessionMcpSelection, SessionMcpTransportKind,
};
use crate::app_event::AppEventPublisher;
use crate::error::{BackendError, ErrorClassification};
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use ora_contracts::{
    AppEvent, EmptyErrorParams, ListMcpHealthRequest, ListMcpHealthResponse, McpHealthEntry,
    McpHealthIdentity as ContractIdentity, McpHealthStatus, McpHealthTransport,
    ProbeMcpHealthRequest, ProbeMcpHealthResponse, PublicError,
};
use ora_domain::{PluginId, SessionId};
use ora_logging::{ora_info, ora_warn};
use ora_utils::mcp::{ProbeError, probe as run_probe};
use probe::bind_probe_transport;
use semver::Version;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Secret-free Host MCP health identity aligned with the Session member revision.
///
/// `cwd` is set only for members that substitute workspace context and only when a real absolute
/// Session cwd is available. Card identities keep `cwd` absent, so a Workspace A result can never
/// be presented as Workspace B's or as the plugin card's.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct McpHealthIdentity {
    pub plugin_id: PluginId,
    pub package_version: Version,
    pub configuration_revision: u64,
    pub transport: SessionMcpTransportKind,
    pub cwd: Option<PathBuf>,
}

impl McpHealthIdentity {
    /// Builds the card or Session identity for one eligible member.
    pub(crate) fn for_member(member: &EligibleMcpMember, cwd: Option<&Path>) -> Self {
        Self {
            plugin_id: member.candidate.plugin_id.clone(),
            package_version: member.candidate.version.clone(),
            configuration_revision: member.configuration_revision,
            transport: member.transport_kind(),
            cwd: if member.needs_workspace_context() {
                cwd.map(Path::to_path_buf)
            } else {
                None
            },
        }
    }

    /// Projects the identity onto the secret-free public DTO.
    fn to_contract(&self) -> ContractIdentity {
        ContractIdentity {
            plugin_id: self.plugin_id.canonical(),
            package_version: self.package_version.to_string(),
            configuration_revision: self.configuration_revision,
            transport: match self.transport {
                SessionMcpTransportKind::Stdio => McpHealthTransport::Stdio,
                SessionMcpTransportKind::Http => McpHealthTransport::Http,
            },
            cwd: self
                .cwd
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

/// Maps transport-level probe failures onto the closed product error-code family.
pub(crate) fn map_probe_error(error: ProbeError) -> ora_contracts::McpHealthErrorCode {
    match error {
        ProbeError::SpawnFailed => ora_contracts::McpHealthErrorCode::McpSpawnFailed,
        ProbeError::ExitedPrematurely => ora_contracts::McpHealthErrorCode::McpExitedPrematurely,
        ProbeError::HandshakeFailed => ora_contracts::McpHealthErrorCode::McpHandshakeFailed,
        ProbeError::Timeout => ora_contracts::McpHealthErrorCode::McpProbeTimeout,
        ProbeError::ToolsUnavailable => ora_contracts::McpHealthErrorCode::McpToolsUnavailable,
        ProbeError::HttpUnreachable => ora_contracts::McpHealthErrorCode::McpHttpUnreachable,
        ProbeError::HttpUnauthorized => ora_contracts::McpHealthErrorCode::McpHttpUnauthorized,
        ProbeError::HttpServerError => ora_contracts::McpHealthErrorCode::McpHttpServerError,
    }
}

fn unknown_not_probed() -> McpHealthStatus {
    McpHealthStatus::Unknown {
        reason: ora_contracts::McpHealthUnknownReason::NotProbed,
    }
}

fn unknown_context_missing() -> McpHealthStatus {
    McpHealthStatus::Unknown {
        reason: ora_contracts::McpHealthUnknownReason::ContextMissing,
    }
}

/// Whether a trigger may reuse a completed result or must attempt a fresh probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeReuse {
    /// Install/save/Session backfill: any completed result for this identity is reused.
    Cached,
    /// User-initiated re-detect: wait on the in-flight attempt or start a new one.
    Force,
}

/// One completed or in-flight probe attempt, shareable across concurrent triggers.
#[derive(Clone)]
struct ProbeAttempt {
    status: Result<McpHealthStatus, SessionMcpError>,
    duration_ms: u64,
}

/// A shared single flight: every concurrent trigger polls the same underlying future and receives
/// the same outcome, whether it joined before or after the probe completed.
type ProbeFlight = Shared<BoxFuture<'static, ProbeAttempt>>;

enum Slot {
    Ready {
        status: McpHealthStatus,
        duration_ms: Option<u64>,
    },
    InFlight(ProbeFlight),
}

struct StoreInner {
    statuses: HashMap<McpHealthIdentity, Slot>,
}

/// Process-local Host MCP health cache with single-flight probes.
#[derive(Clone)]
pub(crate) struct McpHealthStore {
    inner: Arc<Mutex<StoreInner>>,
    events: AppEventPublisher,
    probe_timeout: Duration,
}

/// How one trigger acquires the probe for an identity.
enum Claim {
    /// A completed result this trigger may present as-is.
    Ready(McpHealthStatus),
    /// Another trigger owns the probe; await its shared outcome.
    Join(ProbeFlight),
    /// This trigger owns a new probe.
    Owned,
}

impl McpHealthStore {
    /// Creates an empty store. Restart always starts here: nothing is persisted.
    ///
    /// The probe timeout is a constructor input so tests can exercise the hard-timeout and
    /// single-flight paths without waiting for the production budget.
    pub(crate) fn new(events: AppEventPublisher, probe_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                statuses: HashMap::new(),
            })),
            events,
            probe_timeout,
        }
    }

    /// Returns the completed status for an exact identity, or `Unknown(not_probed)`.
    pub(crate) fn status_for(&self, identity: &McpHealthIdentity) -> McpHealthStatus {
        let guard = self.lock();
        match guard.statuses.get(identity) {
            Some(Slot::Ready { status, .. }) => *status,
            Some(Slot::InFlight(_)) | None => unknown_not_probed(),
        }
    }

    /// Returns a completed result together with the probe duration that produced it.
    fn ready_entry(&self, identity: &McpHealthIdentity) -> Option<(McpHealthStatus, Option<u64>)> {
        let guard = self.lock();
        match guard.statuses.get(identity) {
            Some(Slot::Ready {
                status,
                duration_ms,
            }) => Some((*status, *duration_ms)),
            Some(Slot::InFlight(_)) | None => None,
        }
    }

    /// Drops every cached status for one plugin so uninstall, an update, or a revision change
    /// cannot leave the old identity on any surface.
    pub(crate) fn invalidate_plugin(&self, plugin_id: &PluginId) {
        let changed = {
            let mut guard = self.lock();
            let before = guard.statuses.len();
            guard
                .statuses
                .retain(|identity, _| identity.plugin_id != *plugin_id);
            guard.statuses.len() != before
        };
        if changed {
            self.publish_changed(plugin_id);
        }
    }

    /// Lists health for every currently eligible MCP under the card or Session cwd view.
    pub(crate) fn list(
        &self,
        catalog: &impl SessionMcpCatalog,
        configurations: &impl SessionMcpConfigurationSource,
        request: ListMcpHealthRequest,
    ) -> Result<ListMcpHealthResponse, BackendError> {
        let cwd = parse_optional_absolute_cwd(request.cwd.as_deref())?;
        // The card view presents every eligible member; the Session view narrows by the caller's
        // selection after this workspace-scoped identity projection.
        let members = effective_members(catalog, configurations, &SessionMcpSelection::Automatic)
            .map_err(SessionMcpError::into_backend)?;
        let mut entries = Vec::with_capacity(members.len());
        for member in members {
            let identity = McpHealthIdentity::for_member(&member, cwd.as_deref());
            // A workspace-context member has no representable card identity: it stays
            // `context_missing` rather than being probed against an invented cwd.
            let status = if member.needs_workspace_context() && identity.cwd.is_none() {
                unknown_context_missing()
            } else {
                self.status_for(&identity)
            };
            entries.push(McpHealthEntry {
                identity: identity.to_contract(),
                status,
            });
        }
        Ok(ListMcpHealthResponse { entries })
    }

    /// Runs or joins one probe for an eligible member and waits for its outcome.
    ///
    /// This is the user-initiated re-detect path: it may wait, still under the probe's own hard
    /// timeout. Binding failures become the operation's error because no health result exists for
    /// them; they are resolution failures that would also fail delivery.
    pub(crate) async fn probe(
        &self,
        catalog: &impl SessionMcpCatalog,
        configurations: &impl SessionMcpConfigurationSource,
        request: ProbeMcpHealthRequest,
    ) -> Result<ProbeMcpHealthResponse, BackendError> {
        let plugin_id = PluginId::parse(&request.plugin_id).map_err(|_| {
            BackendError::new(
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
                "mcp health probe plugin id is invalid",
            )
        })?;
        let cwd = parse_optional_absolute_cwd(request.cwd.as_deref())?;
        let member = find_eligible_member(catalog, configurations, &plugin_id)
            .map_err(SessionMcpError::into_backend)?;
        let Some(member) = member else {
            // Configuration-incomplete, uninstalled, or otherwise ineligible plugins are not
            // probed at all; their card keeps showing only the existing configuration state.
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PluginConfigurationNotDeclared(EmptyErrorParams {}),
                "mcp plugin is not eligible for a health probe",
            ));
        };
        let identity = McpHealthIdentity::for_member(&member, cwd.as_deref());
        let status = self
            .ensure_probed(&member, &identity, ProbeReuse::Force)
            .await
            .map_err(SessionMcpError::into_backend)?;
        Ok(ProbeMcpHealthResponse {
            entry: McpHealthEntry {
                identity: identity.to_contract(),
                status,
            },
        })
    }

    /// Fire-and-forget card probe after an install or configuration save made a member eligible.
    ///
    /// The enumeration and probe run on their own task so the install/save response never waits,
    /// and the member is looked up by identity only: which Session selected it is irrelevant.
    pub(crate) fn spawn_card_probe(self, host: SessionMcpHost, plugin_id: PluginId) {
        tokio::spawn(async move {
            let member = match find_eligible_member(&host, &host, &plugin_id) {
                Ok(Some(member)) => member,
                Ok(None) => return,
                Err(error) => {
                    ora_warn!(
                        plugin_id = %plugin_id.canonical(),
                        error = %error,
                        "host MCP health probe could not enumerate eligible members",
                    );
                    return;
                }
            };
            // Installing, updating, or reconfiguring a member changes the card view even when the
            // probe produces no stored result: a workspace-context member short-circuits to
            // `Unknown(context_missing)` and would otherwise never publish on its own, leaving its
            // row hidden behind a cached list. Announce the identity before probing so clients
            // re-query the moment the member becomes eligible.
            self.publish_changed(&plugin_id);
            let identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);
            let _ = self
                .ensure_probed(&member, &identity, ProbeReuse::Cached)
                .await;
        });
    }

    /// Fire-and-forget probes for the members of one Session Effective MCP Set.
    ///
    /// Members whose identity already has a result reuse it and report to the pairing log
    /// immediately; the rest are backfilled and report when their probe settles. An explicit empty
    /// selection yields no members, so this performs no work and the Session shows no banner.
    pub(crate) fn spawn_session_observation<C, S>(
        self,
        catalog: C,
        configurations: S,
        selection: SessionMcpSelection,
        session_id: SessionId,
        cwd: PathBuf,
    ) where
        C: SessionMcpCatalog + Clone + Send + 'static,
        S: SessionMcpConfigurationSource + Clone + Send + 'static,
    {
        tokio::spawn(async move {
            let members = match effective_members(&catalog, &configurations, &selection) {
                Ok(members) => members,
                Err(error) => {
                    ora_warn!(
                        session_id = %session_id,
                        error = %error,
                        "host MCP health probe could not enumerate the Session MCP set",
                    );
                    return;
                }
            };
            for member in members {
                let identity = McpHealthIdentity::for_member(&member, Some(cwd.as_path()));
                // A matching identity already probed in this process is reused as-is; only a
                // member that is still `Unknown(not_probed)` is backfilled.
                if self.ready_entry(&identity).is_none() {
                    let _ = self
                        .ensure_probed(&member, &identity, ProbeReuse::Cached)
                        .await;
                }
                // Pair the settled result with the Session whose setup triggered this observation.
                if let Some((status, duration_ms)) = self.ready_entry(&identity) {
                    log_probe_result(&session_id, &identity, status, duration_ms);
                }
            }
        });
    }

    /// Runs or joins one probe for `identity`, waiting for this trigger's own answer.
    async fn ensure_probed(
        &self,
        member: &EligibleMcpMember,
        identity: &McpHealthIdentity,
        reuse: ProbeReuse,
    ) -> Result<McpHealthStatus, SessionMcpError> {
        // A workspace-context member without a real cwd is never probed against a placeholder.
        if member.needs_workspace_context() && identity.cwd.is_none() {
            return Ok(unknown_context_missing());
        }

        match self.claim(identity, reuse) {
            Claim::Ready(status) => Ok(status),
            // Joining returns the same recorded outcome the owner observes, so a late joiner
            // cannot be told `not_probed` for a probe that already finished.
            Claim::Join(flight) => flight.await.status,
            Claim::Owned => self.run_flight(member, identity).await.status,
        }
    }

    /// Claims the probe for one identity, or reports the result only this trigger may present.
    fn claim(&self, identity: &McpHealthIdentity, reuse: ProbeReuse) -> Claim {
        let guard = self.lock();
        match guard.statuses.get(identity) {
            // Every `Ready` slot holds a completed outcome, so a cached trigger reuses it as-is.
            Some(Slot::Ready { status, .. }) if reuse == ProbeReuse::Cached => {
                Claim::Ready(*status)
            }
            Some(Slot::InFlight(flight)) => Claim::Join(flight.clone()),
            Some(Slot::Ready { .. }) | None => Claim::Owned,
        }
    }

    /// Starts the probe for a claimed identity, records its outcome, and returns the shared result.
    ///
    /// The outcome is recorded from inside the shared future, so when any trigger — the owner or a
    /// joiner, before or after completion — receives an answer, the cache already agrees with it.
    /// A detached task also drives the same future, so a cancelled trigger cannot leave the
    /// identity claimed forever.
    async fn run_flight(
        &self,
        member: &EligibleMcpMember,
        identity: &McpHealthIdentity,
    ) -> ProbeAttempt {
        let member = member.clone();
        let probe_identity = identity.clone();
        let probe_timeout = self.probe_timeout;
        let store = self.clone();
        let flight: ProbeFlight = async move {
            let started = Instant::now();
            let status = match bind_probe_transport(&member, probe_identity.cwd.as_deref()) {
                Ok(transport) => match run_probe(transport, probe_timeout).await {
                    Ok(()) => Ok(McpHealthStatus::Healthy),
                    Err(error) => Ok(McpHealthStatus::Unhealthy {
                        error_code: map_probe_error(error),
                    }),
                },
                // Binding failures are resolution concerns, not runtime health codes: the same
                // failure fails delivery, so no health result is invented for it.
                Err(error) => Err(error),
            };
            let attempt = ProbeAttempt {
                status,
                duration_ms: started.elapsed().as_millis() as u64,
            };
            match &attempt.status {
                Ok(status) => {
                    store.store_ready(&probe_identity, *status, Some(attempt.duration_ms));
                }
                Err(error) => {
                    // Release the claim so a later trigger can retry, and keep the failure in the
                    // operator log only: no public surface has a health result to show for it.
                    store.abandon(&probe_identity);
                    ora_warn!(
                        plugin_id = %probe_identity.plugin_id.canonical(),
                        error = %error,
                        "host MCP health probe could not bind the member",
                    );
                }
            }
            attempt
        }
        .boxed()
        .shared();

        {
            let mut guard = self.lock();
            guard
                .statuses
                .insert(identity.clone(), Slot::InFlight(flight.clone()));
        }

        let driver = flight.clone();
        tokio::spawn(async move {
            let _ = driver.await;
        });
        flight.await
    }

    fn store_ready(
        &self,
        identity: &McpHealthIdentity,
        status: McpHealthStatus,
        duration_ms: Option<u64>,
    ) {
        let changed = {
            let mut guard = self.lock();
            let changed = match guard.statuses.get(identity) {
                Some(Slot::Ready {
                    status: existing, ..
                }) => existing != &status,
                Some(Slot::InFlight(_)) | None => true,
            };
            guard.statuses.insert(
                identity.clone(),
                Slot::Ready {
                    status,
                    duration_ms,
                },
            );
            changed
        };
        if changed {
            self.publish_changed(&identity.plugin_id);
        }
    }

    fn abandon(&self, identity: &McpHealthIdentity) {
        let mut guard = self.lock();
        guard.statuses.remove(identity);
    }

    fn publish_changed(&self, plugin_id: &PluginId) {
        self.events.try_publish(AppEvent::McpHealthChanged {
            plugin_id: plugin_id.canonical(),
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Spawns one bounded, secret-free health observation for a completed Session setup boundary.
///
/// Called after the Session's ACP configuration was logged, so every result it writes can be
/// paired with that same Session. It never blocks and never changes delivery: the caller has
/// already sent its complete `mcpServers` list.
pub(crate) fn observe_session_mcp_health(
    host: &SessionMcpHost,
    session_id: &SessionId,
    cwd: &Path,
) {
    host.mcp_health().spawn_session_observation(
        host.clone(),
        host.clone(),
        host.selection.clone(),
        session_id.clone(),
        cwd.to_path_buf(),
    );
}

/// Structured pairing log for one member's health result: identity, stable code, and duration only.
fn log_probe_result(
    session_id: &SessionId,
    identity: &McpHealthIdentity,
    status: McpHealthStatus,
    duration_ms: Option<u64>,
) {
    let (status_label, code) = match status {
        McpHealthStatus::Healthy => ("healthy", None),
        McpHealthStatus::Unhealthy { error_code } => ("unhealthy", Some(error_code.as_str())),
        McpHealthStatus::Unknown { reason } => (
            "unknown",
            Some(match reason {
                ora_contracts::McpHealthUnknownReason::NotProbed => "not_probed",
                ora_contracts::McpHealthUnknownReason::ContextMissing => "context_missing",
            }),
        ),
    };
    ora_info!(
        session_id = %session_id,
        plugin_id = %identity.plugin_id.canonical(),
        package_version = %identity.package_version,
        configuration_revision = identity.configuration_revision,
        transport = identity.transport.as_str(),
        cwd = identity
            .cwd
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        mcp_health_status = status_label,
        mcp_health_code = code,
        duration_ms,
        "host MCP health probe result"
    );
}

/// Validates the optional card/Session cwd without ever inventing one.
fn parse_optional_absolute_cwd(cwd: Option<&str>) -> Result<Option<PathBuf>, BackendError> {
    let Some(cwd) = cwd else {
        return Ok(None);
    };
    let path = PathBuf::from(cwd);
    if !path.is_absolute() {
        return Err(BackendError::new(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "mcp health cwd must be an absolute path",
        ));
    }
    Ok(Some(path))
}
