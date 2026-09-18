use super::*;
use ora_node_db::Target;
use ora_node_protocol::*;
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

pub struct FixedClock;
impl Clock for FixedClock {
    /// Supplies a local timestamp without touching global logging or environment configuration.
    fn now(&self) -> String {
        "2026-09-10T12:00:00+08:00".into()
    }
}

#[derive(Clone, Default)]
pub struct Faults {
    pub write: Rc<Cell<Option<WritePoint>>>,
    pub git: Rc<Cell<GitFault>>,
    pub calls: Rc<RefCell<Vec<&'static str>>>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum GitFault {
    #[default]
    None,
    Read,
    BeforeCreate,
    AfterCreate,
    BranchOnly,
    BeforeBranch,
    AfterBranch,
}
impl WriteGuard for Faults {
    /// Rejects a named SQLite boundary while leaving actual transaction semantics intact.
    fn before_write(&self, point: WritePoint) -> Result<(), ora_node_db::Error> {
        if self.write.get() == Some(point) {
            Err(ora_node_db::Error::Injected(point))
        } else {
            Ok(())
        }
    }
}

pub struct ControlledGit(pub Faults);
impl WorktreeGit for ControlledGit {
    /// Fails observations deterministically to prove completed retries do not access Git.
    fn main_git_directory(&self, main: &Path) -> Result<PathBuf, WorktreeFailure> {
        if self.0.git.get() == GitFault::Read {
            return Err(crate::resources::failure(
                WorktreeFailureCode::OperationFailed,
                "read failure",
            ));
        }
        gitlancer::Git::new(gitlancer::CliGitRunner).main_git_directory(main)
    }
    /// Uses real ref resolution so tests can move refs and verify the saved commit is retained.
    fn resolve_base(
        &self,
        main: &Path,
        reference: &str,
        branch: &str,
    ) -> Result<CommitId, WorktreeFailure> {
        gitlancer::Git::new(gitlancer::CliGitRunner).resolve_base(main, reference, branch)
    }
    /// Real registration and filesystem observations exercise the production adapter.
    fn observe(&self, target: &Target) -> Result<Observation, WorktreeFailure> {
        gitlancer::Git::new(gitlancer::CliGitRunner).observe(target)
    }
    /// Supports precise simulated process stops immediately before and after the external mutation.
    fn create(&self, target: &Target) -> Result<(), WorktreeFailure> {
        self.0.calls.borrow_mut().push("create");
        if self.0.git.get() == GitFault::BeforeCreate {
            panic!("simulated stop before add");
        }
        if self.0.git.get() == GitFault::BranchOnly {
            cli(
                &target.main_path,
                &[
                    "branch",
                    target.branch.as_str(),
                    target.base_commit.as_str(),
                ],
            );
            return Err(crate::resources::failure(
                WorktreeFailureCode::OperationFailed,
                "partial add",
            ));
        }
        gitlancer::Git::new(gitlancer::CliGitRunner).create(target)?;
        if self.0.git.get() == GitFault::AfterCreate {
            panic!("simulated stop after add");
        }
        Ok(())
    }
    /// Records checkout removal independently from subsequent local branch removal.
    fn remove_worktree(&self, target: &Target) -> Result<(), WorktreeFailure> {
        self.0.calls.borrow_mut().push("remove_worktree");
        gitlancer::Git::new(gitlancer::CliGitRunner).remove_worktree(target)
    }
    /// Can refuse the second deletion stage after the first stage has already committed its intent.
    fn remove_branch(&self, target: &Target) -> Result<(), WorktreeFailure> {
        self.0.calls.borrow_mut().push("remove_branch");
        if self.0.git.get() == GitFault::BeforeBranch {
            return Err(crate::resources::failure(
                WorktreeFailureCode::OperationFailed,
                "branch deletion blocked",
            ));
        }
        gitlancer::Git::new(gitlancer::CliGitRunner).remove_branch(target)?;
        if self.0.git.get() == GitFault::AfterBranch {
            panic!("simulated stop after branch removal");
        }
        Ok(())
    }
    /// Uses nonrecursive directory cleanup to test residual-file protection.
    fn remove_empty_directory(&self, target: &Target) -> Result<(), WorktreeFailure> {
        self.0.calls.borrow_mut().push("remove_empty");
        gitlancer::Git::new(gitlancer::CliGitRunner).remove_empty_directory(target)
    }
}

pub type TestNode = Node<ControlledGit, Faults, FixedClock>;
pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub main: PathBuf,
    pub root: PathBuf,
    pub faults: Faults,
}
impl Fixture {
    /// Provisions an existing main checkout and separate authorized worktree root.
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main");
        let root = directory.path().join("worktrees");
        std::fs::create_dir(&main).unwrap();
        std::fs::create_dir(&root).unwrap();
        cli(&main, &["init", "-b", "main"]);
        cli(
            &main,
            &[
                "-c",
                "user.name=Node Test",
                "-c",
                "user.email=node@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        Self {
            directory,
            main: main.canonicalize().unwrap(),
            root: root.canonicalize().unwrap(),
            faults: Faults::default(),
        }
    }
    /// Reuses the same home and registration while generating a new runtime identity each time.
    pub fn config(&self) -> NodeConfig {
        NodeConfig {
            home_directory: self.directory.path().join("home"),
            identity: NodeIdentity::Discover,
            repositories: vec![RepositoryBinding {
                repository: RepositoryRef::new("repo"),
                main_workspace: MainWorkspaceBinding {
                    workspace_id: WorkspaceId::new("main"),
                    path: NodePath::new(self.main.to_str().unwrap()),
                },
                authorized_root: self.directory.path().to_path_buf(),
                worktree_root: self.root.clone(),
            }],
        }
    }
    /// Reopens the real database with the current fault configuration.
    pub fn open(&self) -> TestNode {
        Node::open_with_dependencies(
            self.config(),
            ControlledGit(self.faults.clone()),
            self.faults.clone(),
            FixedClock,
        )
        .unwrap()
    }
    /// Constructs one stable execution without conflating task and Main Workspace identities.
    pub fn ensure(&self, node: &TestNode) -> Command {
        Command::Ensure(EnsureWorktreeMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request")),
            operation_id: OperationId::new("ensure"),
            execution_id: ExecutionId::new("ensure-execution"),
            payload: EnsureWorktree {
                spec: WorktreeExecutionSpec {
                    node_id: node.node_id().clone(),
                    workspace_id: WorkspaceId::new("task"),
                    worktree_id: WorktreeId::new("tree"),
                    repository: RepositoryRef::new("repo"),
                    main_workspace: self
                        .config()
                        .repositories
                        .remove(/*index*/ 0)
                        .main_workspace,
                    base_ref: GitRef::new("main"),
                    expected_branch: BranchName::new("ora/task"),
                    path_policy: WorktreePathPolicy::NodeManaged {
                        directory_name: "task".into(),
                    },
                },
            },
        })
    }
}

/// Changes only operation/execution identity when requesting removal of the same owned resource.
pub fn removal(command: &Command) -> Command {
    Command::Remove(RemoveWorktreeMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: Some(RequestId::new("remove-request")),
        operation_id: OperationId::new("remove"),
        execution_id: ExecutionId::new("remove-execution"),
        payload: RemoveWorktree {
            spec: command.spec().clone(),
        },
    })
}
/// Builds a status query with the original execution identity and current persistent NodeId.
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
/// Keeps all potentially shared tracing callsites scoped at TRACE during test setup and execution.
pub fn traced(test: impl FnOnce()) {
    let subscriber =
        tracing_subscriber::registry().with(tracing_subscriber::filter::LevelFilter::TRACE);
    tracing::subscriber::with_default(subscriber, test);
}
use tracing_subscriber::prelude::*;
/// Runs fixture-only Git commands without mutating global process environment or user Git config.
pub fn cli(path: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
