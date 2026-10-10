//! Independent Node storage. The open database owns the process-wide execution lease.
mod controller;
mod delivery;
pub use delivery::EventCursor;
mod execution;
mod model;
mod plugin;
mod process;
pub use plugin::PluginExecution;
mod repository;
mod repository_model;
mod revision;
mod runtime_control;
mod schema;
mod session;
pub use execution::owns;
pub use model::*;
pub use process::{ProcessAttempt, ProcessJournal};
pub use repository_model::{CloneExecution, ClonePhase, CloneProgress, CloneTarget};
pub use revision::{DeliveryPlan, DeliveryProgress, FrozenOutcome, RevisionDelivery};
pub use session::{
    CommandAdmission, SessionCommandInput, SessionCommandSettlement, SessionCommandState,
    SessionExecution, SessionJournal,
};

use ora_node_protocol::NodeId;
use ora_utils::fs::{ExclusiveFileLock, ExclusiveLockError};
use rusqlite::Connection;
use std::{fs::OpenOptions, path::Path, sync::Arc};
use thiserror::Error;

/// Storage failures never authorize a caller to proceed with an external mutation.
#[derive(Debug, Error)]
pub enum Error {
    #[error("node database I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("node database SQL: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("node database encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("another runtime owns the node database")]
    AlreadyRunning,
    #[error("unrecognized or unsupported node database")]
    InvalidSchema,
    #[error("configured NodeId does not match the persistent identity")]
    NodeMismatch,
    #[error("controller ownership mismatch or unclaimed historical execution")]
    ControllerMismatch,
    #[error("execution identity conflict")]
    IdentityConflict,
    #[error("resource ownership conflict")]
    ResourceConflict,
    #[error("invalid execution state transition")]
    InvalidTransition,
    #[error("invalid event acknowledgement")]
    InvalidAck,
    #[error("invalid input: {0}")]
    Validation(#[from] ora_node_protocol::MessageValidationError),
    #[error("injected storage failure at {0:?}")]
    Injected(WritePoint),
}

/// Explicit identity policy for bootstrap versus reconnecting a registered Node.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum NodeIdentity {
    Discover,
    Require(NodeId),
}

/// Holds the SQLite connection and an OS lease released automatically on process exit.
pub struct NodeDatabase<G = DurableWrites> {
    guard: Arc<G>,
    connection: Connection,
    node_id: NodeId,
    // A sibling lock inode resolves the same for every spelling of the home, while leaving the
    // database file free for SQLite's own locks: a whole-file lease on the database collides with
    // them on macOS (flock and fcntl share one lock table) and Windows (mandatory LockFileEx).
    _lease: Arc<ExclusiveFileLock>,
}

impl NodeDatabase<DurableWrites> {
    /// Opens only a recognized Node schema; existing foreign files are never initialized.
    pub fn open(path: &Path, identity: NodeIdentity) -> Result<Self, Error> {
        Self::open_with_guard(path, identity, DurableWrites)
    }
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Uses real SQLite with an injectable transaction failure boundary.
    pub fn open_with_guard(path: &Path, identity: NodeIdentity, guard: G) -> Result<Self, Error> {
        if matches!(&identity, NodeIdentity::Require(id) if id.as_str().trim().is_empty()) {
            return Err(Error::NodeMismatch);
        }
        // Creating the file first settles `created` atomically and rejects directories or
        // unwritable paths before any lease is taken beside them.
        let created = match OpenOptions::new()
            .read(/*read*/ true)
            .write(/*write*/ true)
            .create_new(/*create_new*/ true)
            .open(path)
        {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                drop(
                    OpenOptions::new()
                        .read(/*read*/ true)
                        .write(/*write*/ true)
                        .open(path)?,
                );
                false
            }
            Err(error) => return Err(error.into()),
        };
        let Some(name) = path.file_name() else {
            return Err(Error::InvalidSchema);
        };
        let mut lock_name = name.to_os_string();
        lock_name.push(".lock");
        let lease =
            ExclusiveFileLock::try_acquire(&path.with_file_name(lock_name)).map_err(|error| {
                match error {
                    ExclusiveLockError::Busy { .. } => Error::AlreadyRunning,
                    ExclusiveLockError::Io { source, .. } => Error::Io(source),
                }
            })?;
        let mut connection = Connection::open(path)?;
        let node_id = schema::initialize(&mut connection, created, &identity)?;
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "ON")?;
        connection.pragma_update(/*schema_name*/ None, "synchronous", "FULL")?;
        Ok(Self {
            guard: Arc::new(guard),
            connection,
            node_id,
            _lease: Arc::new(lease),
        })
    }

    /// Returns the stable identity without exposing schema mutation APIs.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }
}

#[cfg(test)]
mod tests;
