#![cfg(target_os = "linux")]
use ora_node::{
    Command, Node, NodeConfig, ProcessConfig, RepositoryBinding, WriteGuard, WritePoint,
};
use ora_node_db::NodeIdentity;
use ora_node_protocol::*;
use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process, linux_process_snapshot};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    time::{Duration, Instant},
};

// Each fixture starts several native processes. Bound fixture concurrency independently of
// libtest's CPU-count default, while preserving concurrency exercised inside each test.
const CONCURRENT_PROCESS_FIXTURES: u32 = 4;
static PROCESS_SLOTS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(CONCURRENT_PROCESS_FIXTURES as usize);

pub struct Fixture {
    directory: tempfile::TempDir,
    workload_uid: Option<u32>,
    // Release only after the fixture's children and directory have been torn down.
    _process_slots: tokio::sync::SemaphorePermit<'static>,
}
pub struct ChildGuard(pub Child);
impl ChildGuard {
    /// Simulates abrupt process death and reaps the exact child.
    pub fn kill(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
    /// Exercises the real process's normal-stop path with a bounded observation deadline.
    pub fn terminate(&mut self) {
        LinuxPidFd::from_observation(&linux_process(self.0.id()).unwrap())
            .unwrap()
            .signal(ProcessSignal::Terminate)
            .unwrap();
        until(|| self.0.try_wait().unwrap().is_some());
    }
}
impl Drop for ChildGuard {
    /// Test failures must not leave an independent app behind.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct BeforeMutation;
impl WriteGuard for BeforeMutation {
    /// Seeds a real accepted execution without performing the first Git mutation.
    fn before_write(&self, point: WritePoint) -> Result<(), ora_node_db::Error> {
        if point == WritePoint::Progress {
            Err(ora_node_db::Error::Injected(point))
        } else {
            Ok(())
        }
    }
}

impl Fixture {
    /// Deploys a private Git and explicitly separate Node, repository and HOME paths.
    pub fn new() -> Self {
        Self::with_process_slots(/*slots*/ 1)
    }

    /// Uses a distinct workload identity when this fixture runs with Linux setuid authority.
    pub fn with_workload_uid(uid: u32) -> Self {
        // /root is private, so only the separate-UID fixture uses a traversable parent.
        let directory = tempfile::Builder::new()
            .prefix("n")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in("/")
            .unwrap();
        let mut fixture = Self::with_directory(/*slots*/ 1, directory);
        fs::set_permissions(fixture.path(), fs::Permissions::from_mode(/*mode*/ 0o711)).unwrap();
        fixture.workload_uid = Some(uid);
        fixture
    }

    /// Isolates the command-deadline precondition from unrelated fixture process startup.
    pub fn for_command_deadline() -> Self {
        Self::with_process_slots(CONCURRENT_PROCESS_FIXTURES)
    }

    /// Keeps normal fixtures concurrent while giving deadline tests an explicit scheduling boundary.
    fn with_process_slots(slots: u32) -> Self {
        let parent = PathBuf::from(std::env::var_os("HOME").unwrap())
            .canonicalize()
            .unwrap();
        let directory = tempfile::Builder::new()
            .prefix("n")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(parent)
            .unwrap();
        Self::with_directory(slots, directory)
    }

    /// Bootstraps a fixture in the caller's explicitly selected temporary directory.
    fn with_directory(slots: u32, directory: tempfile::TempDir) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // The fair semaphore lets the deadline fixture reserve the whole budget before
        // bootstrap. Its unchanged 500ms deadline then tests running Git, not admission delay.
        let process_slots = rt.block_on(PROCESS_SLOTS.acquire_many(slots)).unwrap();
        ora_logging::initialize_test_clock();
        let path = directory.path();
        for name in ["main", "trees", "different-home"] {
            fs::create_dir(path.join(name)).unwrap();
        }
        fs::copy("/usr/bin/git", path.join("git")).unwrap();
        let fixture = Self {
            directory,
            workload_uid: None,
            _process_slots: process_slots,
        };
        fixture.git(&["init", "-b", "main"]);
        fixture.git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "--allow-empty",
            "-m",
            "base",
        ]);
        fixture
    }
    /// Returns only this fixture's private deployment root.
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
    /// Supplies the same explicit Node data directory to embedding and executable callers.
    pub fn config(&self) -> NodeConfig {
        NodeConfig {
            home_directory: self.path().join("node"),
            identity: NodeIdentity::Require(NodeId::new("test-node")),
            repositories: vec![RepositoryBinding {
                repository: RepositoryRef::new("repo"),
                main_workspace: MainWorkspaceBinding {
                    workspace_id: WorkspaceId::new("main"),
                    path: NodePath::new(self.path().join("main").to_str().unwrap()),
                },
                authorized_root: self.path().to_path_buf(),
                worktree_root: self.path().join("trees"),
            }],
        }
    }
    /// Isolates Git configuration from the test runner and makes stop deadlines explicit.
    pub fn process(&self) -> ProcessConfig {
        ProcessConfig {
            workload_uid: self.workload_uid,
            host_directory: None,
            expected_uid: None,
            git_program: self.path().join("git"),
            environment: if self.workload_uid.is_some() {
                Default::default()
            } else {
                [
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    (
                        "HOME".into(),
                        self.path().join("different-home").to_str().unwrap().into(),
                    ),
                    ("TEST_DIR".into(), self.path().to_str().unwrap().into()),
                ]
                .into()
            },
            command_timeout_ms: 30_000,
            network_timeout_ms: 30_000,
            cleanup_timeout_ms: 3000,
            shutdown_grace_ms: 100,
        }
    }
    /// Accepts the original business command through Node, failing before its first side effect.
    pub fn seed(&self) -> Command {
        let mut node = Node::open_with_dependencies(
            self.config(),
            gitlancer::Git::new(gitlancer::CliGitRunner),
            BeforeMutation,
            ora_node::LocalClock,
        )
        .unwrap();
        let command = Command::Ensure(EnsureWorktreeMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: None,
            operation_id: OperationId::new("create"),
            execution_id: ExecutionId::new("create-execution"),
            payload: EnsureWorktree {
                spec: WorktreeExecutionSpec {
                    node_id: node.node_id().clone(),
                    workspace_id: WorkspaceId::new("task"),
                    worktree_id: WorktreeId::new("task-tree"),
                    repository: RepositoryRef::new("repo"),
                    main_workspace: self.config().repositories[0].main_workspace.clone(),
                    base_ref: GitRef::new("main"),
                    expected_branch: BranchName::new("ora/task"),
                    path_policy: WorktreePathPolicy::NodeManaged {
                        directory_name: "task".into(),
                    },
                },
            },
        });
        assert!(node.submit(command.clone()).is_err());
        command
    }
    /// A real Git hook exposes a deterministic in-flight write window and an observable late write.
    pub fn install_hook(&self) {
        let hook = self
            .path()
            .join("main")
            .join(".git")
            .join("hooks")
            .join("post-checkout");
        fs::write(&hook, "#!/bin/sh\nprintf '%s' \"$$\" > \"$TEST_DIR/hook-pid\"\nprintf ready > \"$TEST_DIR/hook-ready\"\nwhile test ! -f \"$TEST_DIR/release\"; do /bin/sleep 0.02; done\nprintf late > \"$TEST_DIR/late-write\"\n").unwrap();
        fs::set_permissions(hook, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
    }
    /// The same hook, with its waiting half moved to a session of its own: no process-group
    /// signal reaches it, yet it still holds what it inherited from Git.
    pub fn install_escaping_hook(&self) {
        let hook = self
            .path()
            .join("main")
            .join(".git")
            .join("hooks")
            .join("post-checkout");
        fs::write(&hook, "#!/bin/sh\n/usr/bin/setsid /bin/sh -c 'printf %s \"$$\" > \"$TEST_DIR/hook-pid\"\nprintf ready > \"$TEST_DIR/hook-ready\"\nwhile test ! -f \"$TEST_DIR/release\"; do /bin/sleep 0.02; done\nprintf late > \"$TEST_DIR/late-write\"' &\nwait\n").unwrap();
        fs::set_permissions(hook, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
    }
    /// Starts the production executable with no command channel or Backend composition.
    pub fn launch(&self, label: &str) -> ChildGuard {
        let process = self.process();
        let config = self.path().join(format!("{label}.json"));
        fs::write(&config, serde_json::to_vec(&serde_json::json!({ "node": self.config(), "process": process, "timezone": "Asia/Shanghai", "recovery_interval_ms": 100 })).unwrap()).unwrap();
        let output = fs::File::create(self.path().join(format!("{label}.log"))).unwrap();
        ChildGuard(
            Process::new(env!("CARGO_BIN_EXE_ora-node"))
                .arg(config)
                .env("HOME", self.path().join("different-home"))
                .current_dir(self.path())
                .stdin(Stdio::null())
                .stdout(output)
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        )
    }
    /// Reads human-facing startup/recovery logs only to synchronize tests with the executable.
    pub fn log(&self, label: &str) -> String {
        fs::read_to_string(self.path().join(format!("{label}.log"))).unwrap_or_default()
    }
    /// Independently pins the hook so assertions do not rely only on the Node's own conclusion.
    pub fn pin(&self, name: &str) -> LinuxPidFd {
        let pid = fs::read_to_string(self.path().join(name))
            .unwrap()
            .parse()
            .unwrap();
        LinuxPidFd::from_observation(&linux_process(pid).unwrap()).unwrap()
    }
    /// Executes fixture setup/inspection outside the production mutation path.
    pub fn git(&self, args: &[&str]) -> String {
        let output = Process::new("/usr/bin/git")
            .current_dir(self.path().join("main"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
}
impl Drop for Fixture {
    /// Only processes running from, or inside, this fixture's private directory are signalled:
    /// its Git, the hooks and gates Git started, and whatever of them escaped its process group.
    fn drop(&mut self) {
        for stat in linux_process_snapshot().unwrap().flatten() {
            let proc = Path::new("/proc").join(stat.pid.to_string());
            let inside = |link: &str| {
                fs::read_link(proc.join(link)).is_ok_and(|path| path.starts_with(self.path()))
            };
            if (inside("exe") || inside("cwd"))
                && let Ok(handle) = LinuxPidFd::from_observation(&stat)
            {
                let _ = handle.signal(ProcessSignal::Kill);
            }
        }
    }
}

/// Polls actual externally visible evidence under a failure deadline, never assumes elapsed time proves it.
pub fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 15);
    while !ready() {
        assert!(Instant::now() < deadline, "fixture observation timed out");
        std::thread::sleep(Duration::from_millis(/*millis*/ 10));
    }
}

/// Queries the immutable original business execution identity through Node's production interface.
pub fn query(command: &Command) -> GetExecutionStatusMessage {
    GetExecutionStatusMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id().clone(),
        execution_id: command.execution_id().clone(),
        payload: GetExecutionStatus {
            node_id: command.spec().node_id.clone(),
        },
    }
}

/// Drives one store operation to completion from synchronous test code; the production runtime
/// awaits the same futures, so tests exercise the identical blocking-pool path.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
