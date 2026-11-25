//! In-memory ledger, checkout and plugin catalog, plus a Node home with the echo agent installed.

use ora_node::{
    AgentSessions, CheckoutResolver, CommandSettlement, InstalledConfigurationError,
    ModelProxyConfig, PluginCatalog, PriorRevisionRestore, QueuedCommand, RestoreCancel,
    RestoreFailure, RestoreRequest, Restored, SessionCommand, SessionConfig, SessionHost,
    SessionLedger, SessionWorkload, TurnBoundary, TurnBoundaryPlatform, TurnCancelConfig,
};
use ora_node_protocol::{
    AgentSessionEnded, AgentSessionSpec, BoundModel, CommandId, ContentBlock, ExecutionId,
    FrozenAgentConfiguration, GitIdentity, ModelBindingId, ModelProtocol, NodeId,
    NodeIncarnationId, NodeRuntimeIdentity, OperationId, PluginId, PluginVersion, PriorRevision,
    Sequence, Sha256Digest, ThreadEvent, TurnCancelOutcome, TurnFailureOutcome, TurnId, UserTurn,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

/// The model-gateway fixture the crate's unit tests use, serving the real grant contract.
// Failure toggles exist for the unit tests; these tests use only the default behavior.
#[allow(dead_code)]
#[path = "../../src/model_proxy/test_gateway.rs"]
pub mod gateway;
#[path = "../support/session_stops.rs"]
mod stops;

pub const PLUGIN_ID: &str = "official/ora-space.echo";
pub const PLUGIN_VERSION: &str = "1.0.0";
pub const EXECUTION: &str = "8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41";
pub const CHECKOUT_EXECUTION: &str = "clone-1";
pub const OPERATION: &str = "run-1";

/// Session execution as these tests compose it.
pub type Sessions = AgentSessions<MemoryLedger, Checkouts, Catalog, Restorer>;

/// The failure the in-memory ledger reports.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LedgerError(String);

/// Stops one Thread append until the test releases it, standing in for a crash at that point.
struct AppendGate {
    at: usize,
    fail: bool,
    reached: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

/// One command as the ledger holds it.
struct StoredCommand {
    command_id: CommandId,
    command: SessionCommand,
    settlements: Vec<CommandSettlement>,
}

#[derive(Default)]
struct LedgerState {
    failures: Vec<ora_node_protocol::SessionTurnFailed>,
    events: Vec<ThreadEvent>,
    ended: Option<AgentSessionEnded>,
    commands: Vec<StoredCommand>,
    gate: Option<AppendGate>,
    reject_execution: bool,
    /// Cancel settlements and Thread events in the order they were written.
    journal: Vec<Entry>,
    /// Fails the next end decision once, as a ledger that cannot write would.
    fail_next_decision: bool,
    /// Fails the next queue read once, as a ledger with a passing error would.
    fail_next_queue_read: bool,
    /// Runs when the named command is settled executed, inside the admission commit.
    on_executed: Option<(CommandId, Box<dyn FnOnce() + Send>)>,
    /// Runs when the end decision is recorded, before the Agent is signalled.
    on_decision: Option<Box<dyn FnOnce() + Send>>,
}

/// The keep choices a session starts with.
#[derive(Clone, Copy)]
pub struct Keep {
    pub on_cancel: bool,
    pub on_failure: bool,
}

/// One durable write a test can order against the others.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
    /// A Thread event, named by its record type.
    Thread(String),
    Cancel(String, TurnCancelOutcome),
    /// A failed turn's conclusion, then the cancels merged into its stop.
    Failure(String, TurnFailureOutcome),
}

/// A ledger for one execution that keeps everything in memory.
#[derive(Clone, Default)]
pub struct MemoryLedger {
    state: Arc<Mutex<LedgerState>>,
    stops: stops::Stops,
}

impl MemoryLedger {
    fn state(&self) -> std::sync::MutexGuard<'_, LedgerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Injects failure before a command can become durably Executed.
    pub fn fail_executed_settlements(&self) {
        self.state().reject_execution = true;
    }

    /// Persists one command as the protocol side would before replying that it was accepted.
    pub fn accept(&self, command_id: &str, command: SessionCommand) {
        self.state().commands.push(StoredCommand {
            command_id: CommandId::new(command_id),
            command,
            settlements: Vec::new(),
        });
    }

    /// The end decision recorded for the execution, the one a restart would publish.
    pub fn decision(&self) -> Option<AgentSessionEnded> {
        self.stops.decision(&ExecutionId::new(EXECUTION))
    }

    /// Runs `action` when the end decision is recorded, before the Agent is signalled.
    pub fn on_decision(&self, action: impl FnOnce() + Send + 'static) {
        self.state().on_decision = Some(Box::new(action));
    }

    /// Makes the next queue read fail once.
    pub fn fail_next_queue_read(&self) {
        self.state().fail_next_queue_read = true;
    }

    /// Makes the next end decision fail once.
    pub fn fail_next_decision(&self) {
        self.state().fail_next_decision = true;
    }

    /// Runs `action` when `command` is settled executed, which happens inside the admission commit.
    pub fn on_executed(&self, command: &str, action: impl FnOnce() + Send + 'static) {
        self.state().on_executed = Some((CommandId::new(command), Box::new(action)));
    }

    /// Returns Thread events and cancel settlements in write order.
    pub fn journal(&self) -> Vec<Entry> {
        self.state().journal.clone()
    }

    /// Returns the settlement of each cancel, in settlement order.
    pub fn cancels(&self) -> Vec<(String, TurnCancelOutcome)> {
        self.state()
            .journal
            .iter()
            .filter_map(|entry| match entry {
                Entry::Cancel(command, outcome) => Some((command.clone(), *outcome)),
                Entry::Thread(_) | Entry::Failure(_, _) => None,
            })
            .collect()
    }

    /// Returns every Thread event appended so far.
    pub fn events(&self) -> Vec<ThreadEvent> {
        self.state().events.clone()
    }

    /// Returns the terminal result, once written.
    pub fn ended(&self) -> Option<AgentSessionEnded> {
        self.state().ended.clone()
    }

    /// Returns how each command was settled, in acceptance order.
    pub fn settlements(&self) -> Vec<(String, Vec<CommandSettlement>)> {
        self.state()
            .commands
            .iter()
            .map(|command| (command.command_id.to_string(), command.settlements.clone()))
            .collect()
    }

    /// Makes the append at index `at` wait for the returned release, signalling when it waits.
    pub fn gate_append(&self, at: usize) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        self.pause_append_result(at, true)
    }

    /// Delays a successful append, allowing a durable command to race the ACP result.
    pub fn pause_append(&self, at: usize) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        self.pause_append_result(at, false)
    }

    /// Shares the barrier setup; only the crash fixture injects a failure after release.
    fn pause_append_result(&self, at: usize, fail: bool) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached, reached_receiver) = mpsc::channel();
        let (release_sender, release) = mpsc::channel();
        self.state().gate = Some(AppendGate {
            at,
            fail,
            reached,
            release,
        });
        (reached_receiver, release_sender)
    }
}

impl LedgerState {
    /// Settles every still-queued cancel as `session_ending`, as the end decision does.
    fn settle_pending_cancels(&mut self) {
        for index in 0..self.commands.len() {
            if matches!(self.commands[index].command, SessionCommand::CancelTurn(_))
                && self.commands[index].settlements.is_empty()
            {
                self.commands[index]
                    .settlements
                    .push(CommandSettlement::Executed);
                let command = self.commands[index].command_id.to_string();
                self.journal
                    .push(Entry::Cancel(command, TurnCancelOutcome::SessionEnding));
            }
        }
    }
}

impl SessionLedger for MemoryLedger {
    type Error = LedgerError;
    fn settle_turn_cancel(
        &self,
        _execution: &ExecutionId,
        command_id: &CommandId,
        outcome: TurnCancelOutcome,
        target: Option<&CommandId>,
    ) -> Result<TurnCancelOutcome, Self::Error> {
        let mut state = self.state();
        let Some(index) = state
            .commands
            .iter()
            .position(|command| &command.command_id == command_id)
        else {
            return Err(LedgerError(format!("unknown command {command_id}")));
        };
        let SessionCommand::CancelTurn(turn) = state.commands[index].command.clone() else {
            return Err(LedgerError("not a cancel".into()));
        };
        if let Some(previous) = state.journal.iter().find_map(|entry| match entry {
            Entry::Cancel(command, outcome) if command == command_id.as_str() => Some(*outcome),
            _ => None,
        }) {
            return Ok(previous);
        }
        if outcome != TurnCancelOutcome::SessionEnding
            && state
                .commands
                .iter()
                .any(|command| matches!(command.command, SessionCommand::EndSession(_)))
        {
            return Err(LedgerError("ending".into()));
        }
        let mut outcome = outcome;
        if outcome == TurnCancelOutcome::NotStarted {
            if state.failures.iter().any(|failure| failure.turn_id == turn) {
                outcome = TurnCancelOutcome::AlreadyEnded;
            } else if let Some(target) = target {
                let target = state
                    .commands
                    .iter_mut()
                    .find(|command| &command.command_id == target && command.settlements.is_empty())
                    .ok_or_else(|| LedgerError("target already admitted".into()))?;
                target.settlements.push(CommandSettlement::Discarded);
            }
        }
        state.commands[index]
            .settlements
            .push(CommandSettlement::Executed);
        state
            .journal
            .push(Entry::Cancel(command_id.to_string(), outcome));
        Ok(outcome)
    }
    fn settle_turn_failure(
        &self,
        _execution: &ExecutionId,
        turn: &TurnId,
        outcome: TurnFailureOutcome,
        merged: &[CommandId],
    ) -> Result<(), Self::Error> {
        let mut state = self.state();
        if outcome == TurnFailureOutcome::Kept
            && state
                .commands
                .iter()
                .any(|command| matches!(command.command, SessionCommand::EndSession(_)))
        {
            return Err(LedgerError("ending".into()));
        }
        if state
            .journal
            .iter()
            .any(|entry| matches!(entry, Entry::Failure(turn_id, _) if turn_id == turn.as_str()))
        {
            return Err(LedgerError("already settled".into()));
        }
        state.journal.push(Entry::Failure(turn.to_string(), outcome));
        let cancel_outcome = match outcome {
            TurnFailureOutcome::Kept => TurnCancelOutcome::AlreadyEnded,
            TurnFailureOutcome::SessionEnding => TurnCancelOutcome::SessionEnding,
        };
        for command in merged {
            if let Some(index) = state
                .commands
                .iter()
                .position(|stored| &stored.command_id == command)
            {
                state.commands[index]
                    .settlements
                    .push(CommandSettlement::Executed);
            }
            state
                .journal
                .push(Entry::Cancel(command.to_string(), cancel_outcome));
        }
        Ok(())
    }
    fn fail_session_turn(
        &self,
        _execution: &ExecutionId,
        failure: ora_node_protocol::SessionTurnFailed,
    ) -> Result<(), Self::Error> {
        let mut state = self.state();
        if state
            .commands
            .iter()
            .any(|command| matches!(command.command, SessionCommand::EndSession { .. }))
        {
            return Err(LedgerError("ending".into()));
        }
        if let Some(command_id) = &failure.command_id {
            state
                .commands
                .retain(|command| &command.command_id != command_id);
        }
        state.failures.push(failure);
        Ok(())
    }
    fn record_session_stop(
        &self,
        identity: &ora_node_db::SessionStopIdentity,
        scope: ora_node_db::SessionStopScope,
        at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.stops
            .record(identity, scope, at_ms)
            .map_err(|e| LedgerError(e.to_string()))
    }
    fn session_stop_records(
        &self,
        execution: Option<&ExecutionId>,
    ) -> Result<Vec<ora_node_db::SessionStopRecord>, Self::Error> {
        self.stops
            .records(execution)
            .map_err(|e| LedgerError(e.to_string()))
    }
    fn change_session_stop(
        &self,
        attempt: &ora_node_db::AttemptId,
        change: ora_node_db::SessionStopChange,
    ) -> Result<(), Self::Error> {
        self.stops
            .change(attempt, change)
            .map_err(|e| LedgerError(e.to_string()))
    }
    fn decide_session_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
        at_ms: i64,
        bound_at_ms: i64,
    ) -> Result<(), Self::Error> {
        if std::mem::take(&mut self.state().fail_next_decision) {
            return Err(LedgerError("decision write failed".into()));
        }
        let action = self.state().on_decision.take();
        if let Some(action) = action {
            action();
        }
        self.stops
            .decide(execution, ended, at_ms, bound_at_ms)
            .map_err(|e| LedgerError(e.to_string()))?;
        self.state().settle_pending_cancels();
        Ok(())
    }

    fn append_thread_event(
        &self,
        _execution: &ExecutionId,
        event: ThreadEvent,
    ) -> Result<Sequence, LedgerError> {
        let gated = {
            let mut state = self.state();
            if state.ended.is_some() {
                return Err(LedgerError("the session already ended".to_string()));
            }
            match state.gate.take() {
                Some(gate) if gate.at == state.events.len() => Some(gate),
                other => {
                    state.gate = other;
                    None
                }
            }
        };
        if let Some(gate) = gated {
            let _ = gate.reached.send(());
            let _ = gate.release.recv();
            if gate.fail {
                return Err(LedgerError("the ledger went away".to_string()));
            }
        }
        let mut state = self.state();
        let kind = event
            .record
            .get("type")
            .and_then(|kind| kind.as_str())
            .unwrap_or_default()
            .to_owned();
        state.journal.push(Entry::Thread(kind));
        state.events.push(event);
        Ok(Sequence::new(state.events.len() as u64))
    }

    fn end_session(
        &self,
        _execution: &ExecutionId,
        ended: AgentSessionEnded,
    ) -> Result<Sequence, LedgerError> {
        let mut state = self.state();
        if state.ended.is_some() {
            return Err(LedgerError("the session already ended".to_string()));
        }
        state.settle_pending_cancels();
        state.ended = Some(ended);
        Ok(Sequence::new(state.events.len() as u64 + 1))
    }

    fn queued_commands(&self, _execution: &ExecutionId) -> Result<Vec<QueuedCommand>, LedgerError> {
        if std::mem::take(&mut self.state().fail_next_queue_read) {
            return Err(LedgerError("queue read failed".into()));
        }
        Ok(self
            .state()
            .commands
            .iter()
            .filter(|command| command.settlements.is_empty())
            .map(|command| QueuedCommand {
                command_id: command.command_id.clone(),
                command: command.command.clone(),
            })
            .collect())
    }

    /// Records every settlement, so a test can see a command settled twice.
    fn settle_command(
        &self,
        _execution: &ExecutionId,
        command_id: &CommandId,
        settlement: CommandSettlement,
    ) -> Result<(), LedgerError> {
        let mut state = self.state();
        if state.reject_execution && matches!(settlement, CommandSettlement::Executed) {
            return Err(LedgerError("settlement write failed".into()));
        }
        let command = state
            .commands
            .iter_mut()
            .find(|command| command.command_id == *command_id)
            .ok_or_else(|| LedgerError(format!("unknown command {command_id}")))?;
        command.settlements.push(settlement);
        let hook = match state.on_executed.take() {
            Some((hooked, action))
                if &hooked == command_id && matches!(settlement, CommandSettlement::Executed) =>
            {
                Some(action)
            }
            other => {
                state.on_executed = other;
                None
            }
        };
        drop(state);
        if let Some(action) = hook {
            action();
        }
        Ok(())
    }
}

/// Resolves the one clone execution a test prepared.
pub struct Checkouts(pub HashMap<ExecutionId, PathBuf>);

impl CheckoutResolver for Checkouts {
    fn checkout(&self, clone_execution: &ExecutionId) -> Option<ora_node_db::ClonedCheckout> {
        let path = self.0.get(clone_execution)?.clone();
        let base = "0123456789abcdef0123456789abcdef01234567";
        Some(ora_node_db::ClonedCheckout {
            path,
            spec: ora_node_protocol::CloneExecutionSpec {
                node_id: NodeId::new("node-1"),
                repository_url: ora_node_protocol::GitRepositoryUrl::parse(
                    "https://git.ora.test/tenant/project.git",
                )
                .unwrap(),
                snapshot_ref: ora_node_protocol::SnapshotRef::parse(&format!(
                    "refs/ora/upstreams/9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a/snapshots/{base}"
                ))
                .unwrap(),
                base_commit: ora_node_protocol::CommitId::new(base),
                workspace_ref: ora_node_protocol::WorkspaceRef::of(
                    "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a",
                    "main",
                )
                .unwrap(),
            },
        })
    }
}

/// A lease that counts itself while it lives.
pub struct CountedLease(Arc<AtomicUsize>);

impl Drop for CountedLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Installed versions as the installer would report them, with the leases currently held.
pub struct Catalog {
    pub installed: HashMap<(PluginId, PluginVersion), PathBuf>,
    pub leases: Arc<AtomicUsize>,
    /// How the installed package answers a frozen configuration; accepted unless set.
    pub configuration: Arc<Mutex<Result<(), InstalledConfigurationError>>>,
}

impl PluginCatalog for Catalog {
    type Lease = CountedLease;

    fn lease(&self, _plugin_id: &PluginId) -> CountedLease {
        self.leases.fetch_add(1, Ordering::SeqCst);
        CountedLease(Arc::clone(&self.leases))
    }

    fn installed(&self, plugin_id: &PluginId, version: &PluginVersion) -> Option<PathBuf> {
        self.installed
            .get(&(plugin_id.clone(), version.clone()))
            .cloned()
    }

    fn check_configuration(
        &self,
        _configuration: &FrozenAgentConfiguration,
    ) -> Result<(), InstalledConfigurationError> {
        *self
            .configuration
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Answers every restore with a scripted result and records the requests it saw.
#[derive(Clone)]
pub struct Restorer {
    pub answer: Arc<Mutex<Result<Restored, RestoreFailure>>>,
    pub requests: Arc<Mutex<Vec<RestoreRequest>>>,
    /// While set, a restore runs until it is cancelled, then counts that in `cancelled`.
    pub hold: Arc<AtomicBool>,
    pub cancelled: Arc<AtomicUsize>,
    /// While set, a cancelled hold keeps winding down instead of returning.
    pub stall: Arc<AtomicBool>,
}

impl PriorRevisionRestore for Restorer {
    /// Records the request and answers as scripted.
    async fn restore(
        &self,
        request: RestoreRequest,
        cancel: RestoreCancel,
    ) -> Result<Restored, RestoreFailure> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.hold.load(Ordering::SeqCst) {
            until(|| cancel.requested().then_some(())).await;
            until(|| (!self.stall.load(Ordering::SeqCst)).then_some(())).await;
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            return Err(RestoreFailure::Unavailable);
        }
        self.answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A Node home with the echo agent installed and a Git checkout for it to work in.
pub struct Fixture {
    root: tempfile::TempDir,
    pub ledger: MemoryLedger,
    pub leases: Arc<AtomicUsize>,
    pub restorer: Restorer,
    /// What the catalog answers when a configured session checks its installed package.
    pub configuration: Arc<Mutex<Result<(), InstalledConfigurationError>>>,
    /// Cancel timing and verified Agents for composed sessions; tests adjust it before composing.
    pub turn_cancel: TurnCancelConfig,
}

impl Fixture {
    pub fn new() -> Self {
        ora_logging::initialize_test_clock();
        let root = tempfile::tempdir().expect("create fixture root");
        let fixture = Self {
            root,
            ledger: MemoryLedger::default(),
            turn_cancel: TurnCancelConfig::default(),
            leases: Arc::new(AtomicUsize::new(0)),
            restorer: Restorer {
                answer: Arc::new(Mutex::new(Ok(Restored::OnSnapshotHistory))),
                requests: Arc::default(),
                hold: Arc::default(),
                stall: Arc::default(),
                cancelled: Arc::default(),
            },
            configuration: Arc::new(Mutex::new(Ok(()))),
        };
        let package = fixture.package_root();
        std::fs::create_dir_all(&package).expect("create package root");
        std::fs::write(package.join("main.js"), "export {};\n").expect("write entrypoint");
        std::fs::write(
            package.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"ora-space.echo\"\nkind = \"agent\"\nversion = \"{PLUGIN_VERSION}\"\ndescription = \"Echo agent\"\n"
            ),
        )
        .expect("write manifest");
        std::fs::create_dir_all(fixture.checkout()).expect("create checkout");
        fixture.git(&["init", "--quiet"]);
        // A machine-wide signing policy must not decide whether the fixture can commit.
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture
    }

    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    pub fn checkout(&self) -> PathBuf {
        self.root.path().join("checkout")
    }

    pub fn package_root(&self) -> PathBuf {
        self.package_root_of(PLUGIN_VERSION)
    }

    /// Where the installer places `version` of the echo agent.
    fn package_root_of(&self, version: &str) -> PathBuf {
        self.home()
            .join("plugins")
            .join("installed")
            .join("official")
            .join("ora-space.echo")
            .join(version)
    }

    /// Runs Git in the checkout and returns its standard output.
    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.checkout())
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {args:?} failed: {output:?}");
        String::from_utf8(output.stdout).expect("git output is UTF-8")
    }

    /// The Node incarnation that reports every terminal result.
    pub fn node(&self) -> NodeRuntimeIdentity {
        NodeRuntimeIdentity {
            node_id: NodeId::new("node-1"),
            incarnation_id: NodeIncarnationId::new("incarnation-1"),
        }
    }

    /// Composes session execution over a catalog that reports `installed_version` as installed,
    /// in the directory the installer would give that version; only 1.0.0 is actually there.
    pub fn sessions(&self, installed_version: &str) -> Sessions {
        self.sessions_with(installed_version, SessionWorkload::Shared)
    }

    /// Where a separate workload keeps its session directories.
    pub fn workload_directory(&self) -> PathBuf {
        self.root.path().join("agent")
    }

    /// The fixture's stand-in for `fs.protected_hardlinks`, created protecting hard links; tests
    /// rewrite it to see a host that stopped protecting them.
    pub fn protected_hardlinks(&self) -> PathBuf {
        let sysctl = self.root.path().join("protected_hardlinks");
        if !sysctl.exists() {
            std::fs::write(&sysctl, "1\n").expect("sysctl stand-in");
        }
        sysctl
    }

    /// A separate workload this unprivileged test can run: the test's own ids own the session
    /// home and checkout, and the identity is inherited, since only root may take another.
    pub fn separate_workload(&self) -> SessionWorkload {
        std::fs::create_dir_all(self.workload_directory()).expect("create workload directory");
        let metadata = std::fs::metadata(self.checkout()).expect("checkout metadata");
        SessionWorkload::Separate {
            directory: self.workload_directory(),
            identity: ora_process::ProcessIdentity::Inherit,
            uid: std::os::unix::fs::MetadataExt::uid(&metadata),
            gid: std::os::unix::fs::MetadataExt::gid(&metadata),
            hand_over: ora_node::CheckoutHandOver::new(self.protected_hardlinks()),
        }
    }

    /// Composes session execution as [`Self::sessions`] does, running agents per `workload`.
    pub fn sessions_with(&self, installed_version: &str, workload: SessionWorkload) -> Sessions {
        self.compose(installed_version, workload, /*model_proxy*/ None)
    }

    /// Composes session execution whose model access goes through the gateway at `proxy`.
    pub fn model_sessions(&self, proxy: ModelProxyConfig) -> Sessions {
        self.compose(PLUGIN_VERSION, SessionWorkload::Shared, Some(proxy))
    }

    /// Composes session execution over the fixture's ledger, checkout and catalog.
    fn compose(
        &self,
        installed_version: &str,
        workload: SessionWorkload,
        model_proxy: Option<ModelProxyConfig>,
    ) -> Sessions {
        self.compose_with_ledger(
            installed_version,
            workload,
            model_proxy,
            self.ledger.clone(),
        )
    }

    /// Reuses the same transport fixture with a production SQLite session journal.
    pub fn compose_with_ledger<L: SessionLedger>(
        &self,
        installed_version: &str,
        workload: SessionWorkload,
        model_proxy: Option<ModelProxyConfig>,
        ledger: L,
    ) -> AgentSessions<L, Checkouts, Catalog, Restorer> {
        AgentSessions::new(
            SessionConfig {
                stop: Default::default(),
                home_directory: self.home(),
                deno_path: env!("CARGO_BIN_EXE_ora-node-echo-agent").into(),
                timezone: chrono_tz::UTC,
                agent_ready_timeout: Duration::from_secs(30),
                model_proxy,
                workload,
                turn_cancel: self.turn_cancel.clone(),
            },
            self.node(),
            ledger,
            Checkouts(HashMap::from([(
                ExecutionId::new(CHECKOUT_EXECUTION),
                self.checkout(),
            )])),
            Catalog {
                installed: HashMap::from([
                    (
                        (
                            PluginId::new(PLUGIN_ID),
                            PluginVersion::new(installed_version),
                        ),
                        self.package_root_of(installed_version),
                    ),
                    (
                        (
                            PluginId::new("official/acme.stdio"),
                            PluginVersion::new("1.0.0"),
                        ),
                        self.home()
                            .join("plugins")
                            .join("installed")
                            .join("official")
                            .join("acme.stdio")
                            .join("1.0.0"),
                    ),
                    (
                        (
                            PluginId::new("official/acme.web"),
                            PluginVersion::new("1.0.0"),
                        ),
                        self.home()
                            .join("plugins")
                            .join("installed")
                            .join("official")
                            .join("acme.web")
                            .join("1.0.0"),
                    ),
                ]),
                leases: Arc::clone(&self.leases),
                configuration: Arc::clone(&self.configuration),
            },
            self.restorer.clone(),
        )
    }

    /// Starts the session execution with `initial` as its first turn.
    pub fn start(&self, sessions: &Sessions, version: &str, initial: &str) {
        self.start_resuming(sessions, version, initial, None);
    }

    /// Starts a session that chose to stay after a cancelled and/or a failed turn.
    pub fn start_keeping(&self, sessions: &Sessions, initial: &str, keep: Keep) {
        self.start_keeping_resuming(sessions, initial, keep, None);
    }

    /// Starts a session with keep choices that resumes `prior`, if given.
    pub fn start_keeping_resuming(
        &self,
        sessions: &Sessions,
        initial: &str,
        keep: Keep,
        prior: Option<PriorRevision>,
    ) {
        sessions.start(
            OperationId::new(OPERATION),
            ExecutionId::new(EXECUTION),
            AgentSessionSpec {
                host_mcp_servers: Vec::new(),
                mcp_lifecycle: false,
                keep_session_on_cancel: keep.on_cancel,
                keep_session_on_failure: keep.on_failure,
                mcp: Vec::new(),
                agent_configuration: None,
                node_id: NodeId::new("node-1"),
                agent_plugin_id: PluginId::new(PLUGIN_ID),
                agent_plugin_version: PluginVersion::new(PLUGIN_VERSION),
                checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
                model_binding_id: None,
                git_identity: identity(),
                initial_turn: turn("turn-1", initial),
                prior_revision: prior,
                workspace_directory: vec![],
                trigger: None,
            },
        );
    }

    /// Marks the echo agent's version as having a verified turn boundary and shortens the quiet
    /// window, so a stopped turn can keep the session within a test. The fence and the re-check
    /// get ten seconds: their budgets include the wait for the Node's one stop worker, which the
    /// tests of this binary share, and a busy machine must not turn a clean scope into Unknown.
    pub fn verify_turn_boundary(&mut self) {
        self.turn_cancel.turn_boundary =
            vec![boundary(PLUGIN_VERSION, TurnBoundaryPlatform::Linux)];
        self.turn_cancel.quiet = Duration::from_millis(300);
        self.turn_cancel.process_fence = Duration::from_secs(10);
        self.turn_cancel.process_recheck = Duration::from_secs(10);
    }

    /// Starts the session execution with `initial` as its first turn, resuming `prior` if given.
    pub fn start_resuming(
        &self,
        sessions: &Sessions,
        version: &str,
        initial: &str,
        prior: Option<PriorRevision>,
    ) {
        sessions.start(
            OperationId::new(OPERATION),
            ExecutionId::new(EXECUTION),
            AgentSessionSpec {
                host_mcp_servers: Vec::new(),
                mcp_lifecycle: false,
                keep_session_on_cancel: false,
                keep_session_on_failure: false,
                mcp: Vec::new(),
                agent_configuration: None,
                node_id: NodeId::new("node-1"),
                agent_plugin_id: PluginId::new(PLUGIN_ID),
                agent_plugin_version: PluginVersion::new(version),
                checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
                model_binding_id: None,
                git_identity: identity(),
                initial_turn: turn("turn-1", initial),
                prior_revision: prior,
                workspace_directory: vec![],
                trigger: None,
            },
        );
    }

    /// Starts a session whose adapter receives `settings_json` with the gateway fixture's model.
    pub fn start_configured(&self, sessions: &Sessions, initial: &str, settings_json: &str) {
        self.start_configured_keeping(
            sessions,
            initial,
            settings_json,
            Keep {
                on_cancel: false,
                on_failure: false,
            },
        );
    }

    /// Starts a model-bound session with keep choices.
    pub fn start_configured_keeping(
        &self,
        sessions: &Sessions,
        initial: &str,
        settings_json: &str,
        keep: Keep,
    ) {
        sessions.start(
            OperationId::new(OPERATION),
            ExecutionId::new(EXECUTION),
            AgentSessionSpec {
                host_mcp_servers: Vec::new(),
                mcp_lifecycle: false,
                keep_session_on_cancel: keep.on_cancel,
                keep_session_on_failure: keep.on_failure,
                mcp: Vec::new(),
                agent_configuration: Some(FrozenAgentConfiguration {
                    plugin_id: PluginId::new(PLUGIN_ID),
                    plugin_version: PluginVersion::new(PLUGIN_VERSION),
                    target: "x86_64-unknown-linux-gnu".into(),
                    package_sha256: Sha256Digest::new("a".repeat(64)),
                    schema_version: 2,
                    declaration_fingerprint: Sha256Digest::new("b".repeat(64)),
                    adapter_version: 1,
                    configuration_revision: 4,
                    settings_json: settings_json.into(),
                    protocol: ModelProtocol::OpenaiCompletions,
                    model: BoundModel {
                        id: "team/model".into(),
                        name: "Fixture".into(),
                        context_window: 32768,
                        max_tokens: 1024,
                    },
                }),
                node_id: NodeId::new("node-1"),
                agent_plugin_id: PluginId::new(PLUGIN_ID),
                agent_plugin_version: PluginVersion::new(PLUGIN_VERSION),
                checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
                model_binding_id: Some(ModelBindingId::new("binding-1")),
                git_identity: identity(),
                initial_turn: turn("turn-1", initial),
                prior_revision: None,
                workspace_directory: vec![],
                trigger: None,
            },
        );
    }

    /// Persists one command and wakes the session, as the protocol side does.
    pub fn command(&self, sessions: &Sessions, command_id: &str, command: SessionCommand) {
        self.ledger.accept(command_id, command);
        sessions.command_arrived(&ExecutionId::new(EXECUTION));
    }

    /// Returns the session history exactly as the file holds it, line by line.
    pub fn history_lines(&self) -> Vec<Map<String, Value>> {
        let path =
            ora_history::history_path(&self.home().join("sessions"), EXECUTION).expect("path");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("history line is JSON"))
            .collect()
    }

    /// Returns the process ids of every plugin process the fixture started.
    pub fn plugin_pids(&self) -> Vec<u32> {
        std::fs::read_to_string(self.package_root().join("echo-agent.pids"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.parse().expect("pid"))
            .collect()
    }

    /// Waits for the terminal result.
    pub async fn ended(&self) -> AgentSessionEnded {
        until(|| self.ledger.ended()).await
    }
}

/// The commit identity every session of these tests exports.
pub fn identity() -> GitIdentity {
    GitIdentity {
        name: "Ada Lovelace".to_string(),
        email: "ada@example.com".to_string(),
    }
}

/// Builds one text user turn.
pub fn turn(turn_id: &str, text: &str) -> UserTurn {
    UserTurn {
        turn_id: TurnId::new(turn_id),
        content: vec![ContentBlock::Text {
            text: text.to_string(),
        }],
    }
}

/// Reports whether a process with this id still exists.
pub fn process_exists(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// Polls `probe` until it yields a value, failing the test after a bound no healthy run nears.
pub async fn until<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "condition was not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The echo plugin's entry on the turn-boundary list, for one version and platform.
pub fn boundary(version: &str, platform: TurnBoundaryPlatform) -> TurnBoundary {
    TurnBoundary {
        plugin: PluginId::new(PLUGIN_ID),
        version: PluginVersion::new(version),
        platform,
    }
}
