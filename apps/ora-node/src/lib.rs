//! In-process Node lifecycle; transport and process bootstrapping are separate concerns.
use ora_node_db::{NodeDatabase, NodeIdentity};
use ora_node_protocol::{
    MainWorkspaceBinding, NodeIncarnationId, NodeRuntimeIdentity, RepositoryRef,
};
use std::path::{Path, PathBuf};

/// One explicitly registered repository and its existing Main Workspace.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryBinding {
    pub repository: RepositoryRef,
    pub main_workspace: MainWorkspaceBinding,
    pub authorized_root: PathBuf,
    pub worktree_root: PathBuf,
}

/// Deployment supplies the data directory explicitly; the Node never derives it from HOME.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub home_directory: PathBuf,
    pub identity: NodeIdentity,
    pub repositories: Vec<RepositoryBinding>,
}

mod execution;
mod git;
#[cfg(target_os = "linux")]
mod repository;
mod resources;
pub use git::{Checkout, DirectoryState, ExecutionGitRunner, Observation, WorktreeGit};
#[cfg(target_os = "linux")]
pub use repository::{CloneConfig, CloneSsh};
#[cfg(target_os = "linux")]
mod managed;
#[cfg(target_os = "linux")]
pub use managed::{ManagedGitRunner, ProcessConfig, Shutdown};
#[cfg(target_os = "linux")]
pub type ManagedNode = Node<gitlancer::Git<ManagedGitRunner>>;
pub use ora_node_db::{Command, DurableWrites, WriteGuard, WritePoint};
use ora_node_protocol::*;

/// Supplies local timestamps without requiring tests to mutate process clock configuration.
pub trait Clock {
    /// Returns the local observation timestamp retained with execution progress.
    fn now(&self) -> String;
}
pub struct LocalClock;
impl Clock for LocalClock {
    /// Uses the timezone initialized by the process composition root's Ora logging setup.
    fn now(&self) -> String {
        ora_logging::clock::now_local().to_string()
    }
}

/// Initialization returns errors; live instances distinguish recovery gating from readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    RecoveryPending,
    Ready,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Storage(#[from] ora_node_db::Error),
    #[error(transparent)]
    Validation(#[from] MessageValidationError),
    #[error("node is stopping and cannot accept new work")]
    Stopping,
    #[error("Node shutdown cleanup is unverified: {0}")]
    Shutdown(String),
    #[error("invalid Node configuration: {0}")]
    Configuration(String),
}

/// Exclusive mutable execution access serializes commands and recovery for the leased database.
/// Shared callers can place the Node behind a mutex; concurrent retries wait for the same result.
pub struct Node<G = gitlancer::Git<gitlancer::CliGitRunner>, W = DurableWrites, C = LocalClock> {
    #[cfg(target_os = "linux")]
    repository_config: Option<CloneConfig>,
    home_directory: PathBuf,
    database: NodeDatabase<W>,
    identity: NodeRuntimeIdentity,
    repositories: Vec<RepositoryBinding>,
    git: G,
    clock: C,
    state: NodeState,
}

impl Node {
    /// Composes the explicitly configured host-backed Git adapter under the original Node database lease.
    #[cfg(target_os = "linux")]
    pub fn open(
        config: NodeConfig,
        process: ProcessConfig,
        shutdown: Shutdown,
    ) -> Result<ManagedNode, Error> {
        Self::open_managed_with_dependencies(config, process, shutdown, DurableWrites, LocalClock)
    }

    /// Injects durable-write boundaries and time without replacing the production host-backed adapter.
    #[cfg(target_os = "linux")]
    pub fn open_managed_with_dependencies<W: WriteGuard, C: Clock>(
        config: NodeConfig,
        process: ProcessConfig,
        shutdown: Shutdown,
        writes: W,
        clock: C,
    ) -> Result<Node<gitlancer::Git<ManagedGitRunner<W>>, W, C>, Error> {
        if !config.home_directory.is_absolute()
            || config
                .home_directory
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(Error::Configuration(
                "Node data directory must be absolute without parent traversal".into(),
            ));
        }
        if ora_utils::path::canonicalize_longest_existing_prefix(&config.home_directory)
            == ora_utils::path::canonicalize_longest_existing_prefix(&process.host_directory)
        {
            return Err(Error::Configuration(
                "Node and host require distinct state directories".into(),
            ));
        }
        if config.home_directory.exists() {
            // SAFETY: geteuid only reads process identity; validation never modifies an existing directory.
            ora_utils::path::open_private_path(
                &config.home_directory,
                unsafe { libc::geteuid() },
                ora_utils::path::TrustedPathKind::Directory,
            )
            .map_err(|e| Error::Configuration(e.to_string()))?;
        }
        let node = Node::open_with_dependencies(
            config,
            gitlancer::Git::new(gitlancer::CliGitRunner),
            writes,
            clock,
        )?;
        // Secrets in explicitly supplied Git environments must remain behind an owner-private directory.
        // Never chmod an injected existing path: it may be a user's broader home or another owner's data.
        // SAFETY: geteuid only reads the current process identity.
        ora_utils::path::open_private_path(
            &node.home_directory,
            unsafe { libc::geteuid() },
            ora_utils::path::TrustedPathKind::Directory,
        )
        .map_err(|e| Error::Configuration(e.to_string()))?;
        let runner = ManagedGitRunner::new(node.database.process_journal()?, process, shutdown)
            .map_err(|e| Error::Configuration(e.to_string()))?;
        Ok(Node {
            repository_config: None,
            home_directory: node.home_directory,
            database: node.database,
            identity: node.identity,
            repositories: node.repositories,
            git: gitlancer::Git::new(runner),
            clock: node.clock,
            state: node.state,
        })
    }
}

impl<G: WorktreeGit, W: WriteGuard, C: Clock> Node<G, W, C> {
    /// Normal stop verifies process cleanup but leaves unfinished business reconciliation for restart.
    pub fn shutdown(&mut self) -> Result<(), Error> {
        self.git
            .shutdown()
            .map_err(|failure| Error::Shutdown(failure.message))
    }
    /// Injects Git, real-SQLite write failures and time while retaining the same production state machine.
    pub fn open_with_dependencies(
        config: NodeConfig,
        git: G,
        writes: W,
        clock: C,
    ) -> Result<Self, Error> {
        if !config.home_directory.is_absolute() {
            return Err(Error::Configuration(
                "Node data directory must be an explicit absolute path".into(),
            ));
        }
        for (index, binding) in config.repositories.iter().enumerate() {
            if binding.repository.as_str().trim().is_empty()
                || config.repositories[..index].iter().any(|b| {
                    b.repository == binding.repository
                        || b.main_workspace.workspace_id == binding.main_workspace.workspace_id
                })
            {
                return Err(Error::Configuration(
                    "duplicate or empty repository registration".into(),
                ));
            }
        }
        let mut directory = std::fs::DirBuilder::new();
        directory.recursive(/*recursive*/ true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(/*mode*/ 0o700);
        }
        directory
            .create(&config.home_directory)
            .map_err(ora_node_db::Error::from)?;
        let database = NodeDatabase::open_with_guard(
            &config.home_directory.join("ora-node.sqlite3"),
            config.identity,
            writes,
        )?;
        let identity = NodeRuntimeIdentity {
            node_id: database.node_id().clone(),
            incarnation_id: NodeIncarnationId::new(uuid::Uuid::new_v4().to_string()),
        };
        let state =
            if database.recoverable()?.is_empty() && database.recoverable_clones()?.is_empty() {
                NodeState::Ready
            } else {
                NodeState::RecoveryPending
            };
        Ok(Self {
            #[cfg(target_os = "linux")]
            repository_config: None,
            home_directory: config.home_directory,
            database,
            identity,
            repositories: config.repositories,
            git,
            clock,
            state,
        })
    }
    /// Returns the injected state directory containing ora-node.sqlite3.
    pub fn home_directory(&self) -> &Path {
        &self.home_directory
    }
    /// Reports this runtime, distinct from the origin attached to retained terminal results.
    pub fn identity(&self) -> &NodeRuntimeIdentity {
        &self.identity
    }
    /// Returns the persistent identity owned by the database lease.
    pub fn node_id(&self) -> &NodeId {
        self.database.node_id()
    }
    /// Exposes immutable configuration for inspection.
    pub fn repositories(&self) -> &[RepositoryBinding] {
        &self.repositories
    }
    /// Reports whether pending evidence must be reconciled before new work can begin.
    pub fn state(&self) -> NodeState {
        self.state
    }

    /// Answers from durable evidence only; reads neither Git nor the replay acknowledgement state.
    pub fn status(
        &self,
        query: &GetExecutionStatusMessage,
    ) -> Result<ExecutionStatusMessage, Error> {
        query.validate()?;
        if query.payload.node_id != self.identity.node_id {
            return Err(ora_node_db::Error::NodeMismatch.into());
        }
        let state = self
            .database
            .execution_state(&query.operation_id, &query.execution_id)?;
        Ok(ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: query.operation_id.clone(),
            execution_id: query.execution_id.clone(),
            payload: ExecutionStatus {
                node: self.identity.clone(),
                state,
            },
        })
    }
    /// Returns the original pending event envelopes without changing their delivery state.
    pub fn pending_events(&self) -> Result<Vec<NodeToControllerMessage>, Error> {
        Ok(self.database.pending_events()?)
    }
    /// Removes only an exact acknowledged delivery record, retaining execution input and result.
    pub fn acknowledge(&mut self, ack: &EventAckMessage) -> Result<(), Error> {
        Ok(self.database.acknowledge(ack)?)
    }
}

#[cfg(test)]
mod tests;
