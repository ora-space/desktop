//! Agent session executions: one agent plugin running in one checkout, its settled records
//! relayed as Thread events, and the session commands the ledger queued run in order.
//!
//! Each execution composes its own plugin lifecycle and agent runtime from `ora-agent-runtime`,
//! so it starts only the plugin it names and exports only its own Git identity. A session never
//! outlives the Node process: a restarted Node ends every session it finds unfinished as
//! interrupted instead of resuming it.

mod driver;
mod host;
mod ledger;
mod model;
mod ports;
mod queue;
mod thread;
mod workload;

pub use ports::{
    CheckoutResolver, CommandSettlement, HistoryUnavailable, PluginCatalog, QueuedCommand,
    SessionCommand, SessionHost, SessionLedger,
};
pub use workload::SessionWorkload;
pub(crate) use workload::purge_workload_directory;

use chrono_tz::Tz;
use ora_node_protocol::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionSpec, ExecutionId, NodeRuntimeIdentity,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::Notify;

/// Where and with what a Node runs its session executions.
#[derive(Clone, Debug)]
pub struct SessionConfig {
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
}

/// State shared by every session execution of one Node.
pub(crate) struct Shared<L, C, P> {
    config: SessionConfig,
    node: NodeRuntimeIdentity,
    ledger: L,
    checkouts: C,
    catalog: P,
    /// Wake handles of the sessions running in this process, by execution.
    live: Mutex<HashMap<ExecutionId, Arc<Notify>>>,
    stopping: tokio::sync::watch::Sender<bool>,
    finished: Notify,
    failed: std::sync::atomic::AtomicBool,
}

impl<L, C, P> Shared<L, C, P> {
    /// Session histories live beside the plugin root in the Node data directory.
    fn sessions_root(&self) -> PathBuf {
        self.config.home_directory.join("sessions")
    }

    /// Forgets a session whose history nothing writes to anymore.
    fn release(&self, execution: &ExecutionId) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(execution);
        self.finished.notify_waiters();
    }

    /// Reports whether a session of this execution is running in this process.
    fn is_live(&self, execution: &ExecutionId) -> bool {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(execution)
    }
}

/// Runs the Node's Agent session executions.
pub struct AgentSessions<L, C, P> {
    shared: Arc<Shared<L, C, P>>,
}

impl<L, C, P> Clone for AgentSessions<L, C, P> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<L, C, P> AgentSessions<L, C, P>
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
{
    /// Composes session execution over the ledger, clone bookkeeping and plugin catalog of the
    /// Node incarnation `node`, which every terminal result reports.
    pub fn new(
        config: SessionConfig,
        node: NodeRuntimeIdentity,
        ledger: L,
        checkouts: C,
        catalog: P,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                node,
                ledger,
                checkouts,
                catalog,
                live: Mutex::new(HashMap::new()),
                stopping: tokio::sync::watch::channel(/*init*/ false).0,
                finished: Notify::new(),
                failed: std::sync::atomic::AtomicBool::new(/*v*/ false),
            }),
        }
    }
}

impl<L, C, P> AgentSessions<L, C, P> {
    /// Model-bound sessions require this additional negotiated capability.
    pub(crate) fn model_capable(&self) -> bool {
        self.shared.config.model_proxy.is_some()
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

impl<L, C, P> SessionHost for AgentSessions<L, C, P>
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
{
    /// Spawns the session on the current Tokio runtime; a repeated start of a live execution is
    /// ignored, because its input is persisted once and the running session already owns it.
    fn start(&self, execution: ExecutionId, spec: AgentSessionSpec) {
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
        }
        let mut completion = Completion {
            shared: Arc::clone(&self.shared),
            execution: execution.clone(),
            committed: false,
        };
        tokio::spawn(async move {
            completion.committed =
                driver::run(Arc::clone(&completion.shared), execution, spec, wake).await;
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
        }
    }

    fn sealed_history(&self, execution: &ExecutionId) -> Result<PathBuf, HistoryUnavailable> {
        if self.shared.is_live(execution) {
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
struct Completion<L, C, P> {
    shared: Arc<Shared<L, C, P>>,
    execution: ExecutionId,
    committed: bool,
}
impl<L, C, P> Drop for Completion<L, C, P> {
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
