//! In-memory ledger, checkout and plugin catalog, plus a Node home with the echo agent installed.

use ora_node::{
    AgentSessions, CheckoutResolver, CommandSettlement, PluginCatalog, QueuedCommand,
    SessionCommand, SessionConfig, SessionHost, SessionLedger, SessionWorkload,
};
use ora_node_protocol::{
    AgentSessionEnded, AgentSessionSpec, CommandId, ContentBlock, ExecutionId, GitIdentity, NodeId,
    NodeIncarnationId, NodeRuntimeIdentity, PluginId, PluginVersion, Sequence, ThreadEvent, TurnId,
    UserTurn,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

pub const PLUGIN_ID: &str = "official/ora-space.echo";
pub const PLUGIN_VERSION: &str = "1.0.0";
pub const EXECUTION: &str = "8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41";
pub const CHECKOUT_EXECUTION: &str = "clone-1";

/// The failure the in-memory ledger reports.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LedgerError(String);

/// Stops one Thread append until the test releases it, standing in for a crash at that point.
struct AppendGate {
    at: usize,
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
    events: Vec<ThreadEvent>,
    ended: Option<AgentSessionEnded>,
    commands: Vec<StoredCommand>,
    gate: Option<AppendGate>,
    reject_execution: bool,
}

/// A ledger for one execution that keeps everything in memory.
#[derive(Clone, Default)]
pub struct MemoryLedger {
    state: Arc<Mutex<LedgerState>>,
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
        let (reached, reached_receiver) = mpsc::channel();
        let (release_sender, release) = mpsc::channel();
        self.state().gate = Some(AppendGate {
            at,
            reached,
            release,
        });
        (reached_receiver, release_sender)
    }
}

impl SessionLedger for MemoryLedger {
    type Error = LedgerError;

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
            return Err(LedgerError("the ledger went away".to_string()));
        }
        let mut state = self.state();
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
        state.ended = Some(ended);
        Ok(Sequence::new(state.events.len() as u64 + 1))
    }

    fn queued_commands(&self, _execution: &ExecutionId) -> Result<Vec<QueuedCommand>, LedgerError> {
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
        Ok(())
    }
}

/// Resolves the one clone execution a test prepared.
pub struct Checkouts(pub HashMap<ExecutionId, PathBuf>);

impl CheckoutResolver for Checkouts {
    fn checkout(&self, clone_execution: &ExecutionId) -> Option<PathBuf> {
        self.0.get(clone_execution).cloned()
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
}

/// A Node home with the echo agent installed and a Git checkout for it to work in.
pub struct Fixture {
    root: tempfile::TempDir,
    pub ledger: MemoryLedger,
    pub leases: Arc<AtomicUsize>,
}

impl Fixture {
    pub fn new() -> Self {
        ora_logging::initialize_test_clock();
        let root = tempfile::tempdir().expect("create fixture root");
        let fixture = Self {
            root,
            ledger: MemoryLedger::default(),
            leases: Arc::new(AtomicUsize::new(0)),
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
    pub fn sessions(
        &self,
        installed_version: &str,
    ) -> AgentSessions<MemoryLedger, Checkouts, Catalog> {
        self.sessions_with(installed_version, SessionWorkload::Shared)
    }

    /// Where a separate workload keeps its session directories.
    pub fn workload_directory(&self) -> PathBuf {
        self.root.path().join("agent")
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
        }
    }

    /// Composes session execution as [`Self::sessions`] does, running agents per `workload`.
    pub fn sessions_with(
        &self,
        installed_version: &str,
        workload: SessionWorkload,
    ) -> AgentSessions<MemoryLedger, Checkouts, Catalog> {
        AgentSessions::new(
            SessionConfig {
                home_directory: self.home(),
                deno_path: env!("CARGO_BIN_EXE_ora-node-echo-agent").into(),
                timezone: chrono_tz::UTC,
                agent_ready_timeout: Duration::from_secs(30),
                model_proxy: None,
                workload,
            },
            self.node(),
            self.ledger.clone(),
            Checkouts(HashMap::from([(
                ExecutionId::new(CHECKOUT_EXECUTION),
                self.checkout(),
            )])),
            Catalog {
                installed: HashMap::from([(
                    (
                        PluginId::new(PLUGIN_ID),
                        PluginVersion::new(installed_version),
                    ),
                    self.package_root_of(installed_version),
                )]),
                leases: Arc::clone(&self.leases),
            },
        )
    }

    /// Starts the session execution with `initial` as its first turn.
    pub fn start(
        &self,
        sessions: &AgentSessions<MemoryLedger, Checkouts, Catalog>,
        version: &str,
        initial: &str,
    ) {
        sessions.start(
            ExecutionId::new(EXECUTION),
            AgentSessionSpec {
                node_id: NodeId::new("node-1"),
                agent_plugin_id: PluginId::new(PLUGIN_ID),
                agent_plugin_version: PluginVersion::new(version),
                checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
                model_binding_id: None,
                git_identity: identity(),
                initial_turn: turn("turn-1", initial),
            },
        );
    }

    /// Persists one command and wakes the session, as the protocol side does.
    pub fn command(
        &self,
        sessions: &AgentSessions<MemoryLedger, Checkouts, Catalog>,
        command_id: &str,
        command: SessionCommand,
    ) {
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
