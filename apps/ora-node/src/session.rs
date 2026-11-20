//! Agent session executions: one agent plugin running in one checkout, its settled records
//! relayed as Thread events, and the session commands the ledger queued run in order.
//!
//! Each execution composes its own plugin lifecycle and agent runtime from `ora-agent-runtime`,
//! so it starts only the plugin it names and exports only its own Git identity. A session never
//! resumes after Node restart. Original restore stop evidence must precede interrupted settlement;
//! an unknown original process retains its session and checkout responsibility.

mod driver;
mod end;
mod fence;
mod host;
mod ledger;
mod mcp;
mod model;
mod ports;
mod preview;
mod process;
mod queue;
mod resume;
mod scheduled;
pub(crate) mod stop;
mod thread;
mod workload;
pub(crate) use workload::storage::validate as validate_workload_storage;

pub(crate) use model::seal_unstarted_history;
pub use ports::{
    CheckoutResolver, CommandSettlement, HistoryUnavailable, InstalledConfigurationError,
    PluginCatalog, PriorRevisionRestore, QueuedCommand, RestoreCancel, RestoreFailure,
    RestoreRequest, Restored, SessionCommand, SessionHost, SessionLedger,
};
pub use workload::SessionWorkload;
pub(crate) use workload::purge_workload_directory;

use chrono_tz::Tz;
use ora_node_protocol::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionSpec, CommandId, ExecutionId,
    NodeRuntimeIdentity, OperationId,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::Notify;

/// Where and with what a Node runs its session executions.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub stop: crate::SessionStopConfig,
    /// The Node data directory: agent plugins are installed under its `plugins/` root and session
    /// histories are kept under `sessions/`.
    pub home_directory: PathBuf,
    /// The Deno executable agent plugins run on.
    pub deno_path: PathBuf,
    /// The timezone scheduled runtime work is evaluated in.
    pub timezone: Tz,
    /// How long a session waits for its agent to become ready before it ends as failed.
    pub agent_ready_timeout: Duration,
    /// Available only in deployments provisioned for platform model access.
    pub model_proxy: Option<crate::ModelProxyConfig>,
    /// Which OS identity agents run as, and where their per-session directories live.
    pub workload: SessionWorkload,
    /// Timing and Agent eligibility for keeping a session after a turn is cancelled or fails.
    pub turn_cancel: TurnCancelConfig,
}

/// Parameters of D2 in the cancel-turn decision; both durations are provisional until measured.
#[derive(Clone, Debug)]
pub struct TurnCancelConfig {
    /// G: how long a `CancelTurn` waits for the Agent to answer, from its acceptance.
    pub grace: Duration,
    /// Q: how long a stopped turn must stay silent before the next turn is sent.
    pub quiet: Duration,
    /// Agent plugin versions whose turn boundary a conformance run verified. A cancelled or
    /// failed turn of any other Agent ends the session with `turn_boundary_unverified`.
    pub turn_boundary: Vec<(
        ora_node_protocol::PluginId,
        ora_node_protocol::PluginVersion,
    )>,
}

impl Default for TurnCancelConfig {
    /// G 15 s and Q 2 s as first set by the decision; no Agent is verified yet, so every stop of
    /// a running turn ends the session until conformance runs add versions.
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(15),
            quiet: Duration::from_secs(2),
            turn_boundary: Vec::new(),
        }
    }
}

/// State shared by every session execution of one Node.
pub(crate) struct Shared<L, C, P: PluginCatalog, R> {
    config: SessionConfig,
    node: NodeRuntimeIdentity,
    ledger: L,
    checkouts: C,
    catalog: P,
    /// Restores the prior Revision of a resumed session before its agent starts.
    restorer: R,
    /// Publishes each session's `ora-git` socket or the configuration reason it is disabled.
    ora_git: Option<crate::ora_git::Availability>,
    /// Wake handles of the sessions running in this process, by execution.
    live: Mutex<HashMap<ExecutionId, Arc<Notify>>>,
    /// When each pending `CancelTurn` was durably accepted: a stop's deadline counts from the
    /// first cancel's acceptance, not from when the conversation got to it (D1).
    cancel_acceptance: Mutex<HashMap<(ExecutionId, CommandId), tokio::time::Instant>>,
    stopping: tokio::sync::watch::Sender<bool>,
    finished: Notify,
    failed: std::sync::atomic::AtomicBool,
    previews: preview::PreviewBuffer,
    retained: Mutex<HashMap<ExecutionId, stop::Retained<P::Lease>>>,
    controls: Mutex<HashMap<ExecutionId, mcp::control::Control>>,
    probes: mcp::probes::Scheduler,
}

impl<L, C, P: PluginCatalog, R> Shared<L, C, P, R> {
    /// Closes every spawn clone before releasing Node's retained OFD copy.
    fn stop_spawning(&self, execution: &ExecutionId) {
        if let Some(retained) = self
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
        {
            retained.spawner.close();
        }
    }

    /// Proof precedes view deletion and dropping the exact package leases.
    fn release_resources(&self, execution: &ExecutionId) -> std::io::Result<()> {
        // Both live completion and automatic recovery must wake durable probe owners.
        self.probes.1.notify_waiters();
        let mut resources = self.retained.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(retained) = resources.get_mut(execution) {
            retained.placement.release()?;
            retained.leases.clear();
        }
        resources.remove(execution);
        self.controls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(execution);
        Ok(())
    }
    /// Session histories live beside the plugin root in the Node data directory.
    fn sessions_root(&self) -> PathBuf {
        self.config.home_directory.join("sessions")
    }

    /// Forgets a session whose history nothing writes to anymore.
    fn release(&self, execution: &ExecutionId) {
        self.previews.remove(execution);
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(execution);
        self.finished.notify_waiters();
    }

    /// Reports whether a session of this execution is running in this process.
    pub(crate) fn is_live(&self, execution: &ExecutionId) -> bool {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(execution)
    }
}

/// Runs the Node's Agent session executions.
pub struct AgentSessions<L, C, P: PluginCatalog, R> {
    shared: Arc<Shared<L, C, P, R>>,
}

impl<L, C, P: PluginCatalog, R> Clone for AgentSessions<L, C, P, R> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<L, C, P, R> AgentSessions<L, C, P, R>
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    /// Composes session execution over the ledger, clone bookkeeping, plugin catalog and prior
    /// Revision restore of the Node incarnation `node`, which every terminal result reports.
    pub fn new(
        config: SessionConfig,
        node: NodeRuntimeIdentity,
        ledger: L,
        checkouts: C,
        catalog: P,
        restorer: R,
    ) -> Self {
        let probes = mcp::probes::Scheduler::default();
        let recovered = ledger
            .session_stop_records(None)
            .map(|records| {
                records
                    .into_iter()
                    .filter(|record| {
                        matches!(
                            record.identity.kind,
                            ora_node_db::SessionStopKind::StdioProbe
                                | ora_node_db::SessionStopKind::HttpProbe
                        ) && !record.resources_released
                    })
                    .count()
            })
            .unwrap_or(8);
        probes.retain_recovered(recovered);
        Self {
            shared: Arc::new(Shared {
                config,
                node,
                ledger,
                checkouts,
                catalog,
                restorer,
                ora_git: None,
                live: Mutex::new(HashMap::new()),
                cancel_acceptance: Mutex::new(HashMap::new()),
                stopping: tokio::sync::watch::channel(/*init*/ false).0,
                finished: Notify::new(),
                failed: std::sync::atomic::AtomicBool::new(/*v*/ false),
                previews: preview::PreviewBuffer::default(),
                retained: Mutex::new(HashMap::new()),
                controls: Mutex::new(HashMap::new()),
                probes,
            }),
        }
    }

    /// Prior-incarnation probes stop consuming permits only after recovery proves them stopped.
    pub(crate) fn refresh_probe_capacity(&self) {
        let count = self
            .shared
            .ledger
            .session_stop_records(None)
            .map(|records| {
                records
                    .into_iter()
                    .filter(|record| {
                        record.identity.node != self.shared.node
                            && matches!(
                                record.identity.kind,
                                ora_node_db::SessionStopKind::StdioProbe
                                    | ora_node_db::SessionStopKind::HttpProbe
                            )
                            && !record.resources_released
                    })
                    .count()
            })
            .unwrap_or(8);
        self.shared.probes.retain_recovered(count);
        self.shared.probes.dispatch();
    }
}

/// Targets Cloud accepts in an adapter capability; advertising any other would make Cloud refuse
/// the whole registration instead of only this capability.
const ADAPTER_TARGETS: [&str; 2] = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

impl<L, C, P: PluginCatalog, R> AgentSessions<L, C, P, R> {
    /// Capacity is anonymous even when another execution is retaining cleanup responsibility.
    pub(crate) fn probe_capacity_available(&self) -> bool {
        self.shared.probes.available()
    }
    /// Shares the diagnostic and explicit-reconciliation boundary with the local admin socket.
    pub(crate) fn probe_scheduler(&self) -> mcp::probes::Scheduler {
        self.shared.probes.clone()
    }
    /// The service must authorize execution and control scope before taking this memory handle.
    pub(crate) fn mcp_control(&self, execution: &ExecutionId) -> Option<mcp::control::Control> {
        self.shared
            .controls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
            .cloned()
    }
    /// Reconciliation releases resources retained by an actor after its stop deadline expired.
    pub(crate) fn release_stop_resources(&self, execution: &ExecutionId) -> std::io::Result<()> {
        self.shared.release_resources(execution)?;
        if let SessionWorkload::Separate { directory, .. } = &self.shared.config.workload {
            let view = directory.join(ora_utils::hash::sha256_hex(execution.as_str().as_bytes()));
            match std::fs::remove_dir_all(view) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    /// Gives every session an `ora-git` socket or its disabled reason. Composition calls it before
    /// any session starts, while it still holds the only handle.
    pub(crate) fn with_ora_git(mut self, broker: crate::ora_git::Availability) -> Self {
        if let Some(shared) = Arc::get_mut(&mut self.shared) {
            shared.ora_git = Some(broker);
        }
        self
    }

    /// Recovery must never compete with the actor currently executing restore.
    pub(crate) fn is_live(&self, execution: &ExecutionId) -> bool {
        self.shared.is_live(execution)
    }

    /// The transport flushes replacements independently of durable event replay and Ack.
    pub fn preview_snapshots(&self) -> Vec<ora_node_protocol::ThreadPreviewMessage> {
        self.shared.previews.snapshots()
    }
    /// Model-bound sessions require this additional negotiated capability.
    pub(crate) fn model_capable(&self) -> bool {
        self.shared.config.model_proxy.is_some()
    }

    /// The adapter path needs model access and a target plugin packages are built for; a host
    /// outside the supported Linux targets keeps only the legacy OpenCode route.
    pub(crate) fn model_adapter(&self) -> Option<ora_node_protocol::ModelAdapterCapability> {
        let target = ora_plugin_registry::current_host_target()?;
        (self.model_capable() && ADAPTER_TARGETS.contains(&target.as_str())).then(|| {
            ora_node_protocol::ModelAdapterCapability {
                contract_version: ora_node_protocol::MODEL_ADAPTER_CONTRACT_VERSION,
                target: target.as_str().to_string(),
                protocols: vec![
                    ora_node_protocol::ModelProtocol::OpenaiCompletions,
                    ora_node_protocol::ModelProtocol::AnthropicMessages,
                ],
            }
        })
    }
    /// A lost actor or failed terminal write stops admission instead of stranding a Running row.
    pub(crate) fn failed(&self) -> bool {
        self.shared
            .failed
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Stops conversations cooperatively and waits for plugin cleanup and terminal persistence.
    pub async fn shutdown(&self) {
        self.shared.stopping.send_replace(/*value*/ true);
        loop {
            let finished = self.shared.finished.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if self
                .shared
                .live
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
            {
                break;
            }
            finished.await;
        }
    }
}

impl<L: SessionLedger, C, P: PluginCatalog, R> AgentSessions<L, C, P, R> {
    /// Records that a `CancelTurn` was durably accepted at `accepted_at` and wakes the session at
    /// once. The protocol side calls this right after the ledger commit, before any reply is
    /// written, so a reply that cannot be delivered never delays the settlement.
    ///
    /// The time is kept only while the cancel is still queued: a duplicate of a settled cancel,
    /// or one the ledger settled while accepting it, leaves nothing behind. The session removes
    /// the entry when it settles the cancel and clears whatever is left when it ends.
    pub fn turn_cancel_accepted(
        &self,
        execution: &ExecutionId,
        command: &CommandId,
        accepted_at: tokio::time::Instant,
    ) {
        let queued = self
            .shared
            .ledger
            .queued_commands(execution)
            .is_ok_and(|queued| {
                queued.iter().any(|queued| {
                    &queued.command_id == command
                        && matches!(queued.command, SessionCommand::CancelTurn(_))
                })
            });
        if queued {
            self.shared
                .cancel_acceptance
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry((execution.clone(), command.clone()))
                .or_insert(accepted_at);
        }
        if let Some(wake) = self
            .shared
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
        {
            wake.notify_one();
        }
    }

    /// How many cancel acceptance times are held, for diagnostics and tests: each belongs to a
    /// cancel that is accepted and not yet settled.
    pub fn pending_cancel_acceptances(&self) -> usize {
        self.shared
            .cancel_acceptance
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl<L, C, P, R> SessionHost for AgentSessions<L, C, P, R>
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    /// Spawns the session on the current Tokio runtime; a repeated start of a live execution is
    /// ignored, because its input is persisted once and the running session already owns it.
    fn start(&self, operation: OperationId, execution: ExecutionId, spec: AgentSessionSpec) {
        let wake = Arc::new(Notify::new());
        {
            let mut live = self
                .shared
                .live
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if live.contains_key(&execution) {
                return;
            }
            live.insert(execution.clone(), Arc::clone(&wake));
            if spec.mcp_lifecycle {
                self.shared
                    .controls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(execution.clone(), mcp::control::Control::new(&spec.mcp));
            }
        }
        let mut completion = Completion {
            shared: Arc::clone(&self.shared),
            execution: execution.clone(),
            committed: false,
        };
        tokio::spawn(async move {
            completion.committed = driver::run(
                Arc::clone(&completion.shared),
                operation,
                execution,
                spec,
                wake,
            )
            .await;
            drop(completion);
        });
    }

    /// A wake that finds no live session is dropped: the session already ended, and the ledger
    /// settles whatever it left queued with the terminal result.
    fn command_arrived(&self, execution: &ExecutionId) {
        if let Some(wake) = self
            .shared
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
        {
            wake.notify_one();
        }
    }

    /// No agent runtime survives a Node restart: the history's only writer ended with the old
    /// process, so what the file holds is final and there is nothing to resume or seal further.
    /// The plugin's stdio ended with the old process too, which ends a well-behaved plugin; a
    /// descendant that ignores that is not reclaimed until process containment covers plugins.
    fn recover_interrupted(&self, _execution: &ExecutionId) -> AgentSessionEnded {
        AgentSessionEnded {
            node: self.shared.node.clone(),
            reason: AgentSessionEndReason::Interrupted,
            detail: None,
            plugin_id: None,
            setting_id: None,
            transport: None,
            scheduled_result: None,
            artifact_manifest: None,
        }
    }

    fn sealed_history(&self, execution: &ExecutionId) -> Result<PathBuf, HistoryUnavailable> {
        if self.shared.is_live(execution)
            || !self
                .shared
                .ledger
                .session_completed(execution)
                .unwrap_or(/*default*/ false)
        {
            return Err(HistoryUnavailable);
        }
        let path = ora_history::history_path(&self.shared.sessions_root(), execution.as_str())
            .map_err(|_invalid| HistoryUnavailable)?;
        if path.is_file() {
            Ok(path)
        } else {
            Err(HistoryUnavailable)
        }
    }
}

/// Task cancellation and panic release waiters too; recovery, not a second actor, owns any missing terminal.
struct Completion<L, C, P: PluginCatalog, R> {
    shared: Arc<Shared<L, C, P, R>>,
    execution: ExecutionId,
    committed: bool,
}
impl<L, C, P: PluginCatalog, R> Drop for Completion<L, C, P, R> {
    /// Runtime teardown must never leave the blocking service waiting for an aborted actor.
    fn drop(&mut self) {
        if !self.committed {
            self.shared
                .failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        self.shared.release(&self.execution);
    }
}
