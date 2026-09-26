//! Local durable clone coordination; no Desktop/Backend writer or Cloud authority is installed.
//! Coordination logic reaches persistence only through [`CoordinationStore`]: the SQLite adapter
//! under `sqlite` for local deployments, the Cloud RPC adapter under `cloud` for cloud ones.
#[cfg(target_os = "linux")]
mod api;
#[cfg(target_os = "linux")]
mod cloud;
mod coordination;
#[cfg(target_os = "linux")]
mod deployment;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod service;
#[cfg(target_os = "linux")]
mod session;
#[cfg(target_os = "linux")]
mod single_node;
mod sqlite;
mod store;
#[cfg(target_os = "linux")]
mod transport;
#[cfg(target_os = "linux")]
pub use cloud::CloudStore;
pub use coordination::take_over;
#[cfg(target_os = "linux")]
pub use deployment::{ApiConfig, DeploymentConfig, NodeHosting, SingleNodeConfig};
use ora_node_protocol::*;
#[cfg(target_os = "linux")]
pub use runtime::{ControllerHandle, ControllerRuntime, Persistence, RuntimeConfig};
#[cfg(target_os = "linux")]
pub use service::Service;
#[cfg(target_os = "linux")]
pub use session::{
    NodeEndpoint, NodeTarget, SessionConfig, SessionError, run_session, run_session_until,
};
pub use sqlite::SqliteStore;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
pub use store::{CloneIntake, CoordinationStore, ExecutionOutcome};
#[cfg(target_os = "linux")]
pub use transport::{DEFAULT_PORT, Listener, Transport};

/// Persistence failures never authorize dispatch or acknowledgement. The classes an adapter must
/// distinguish are fixed here: a conflict is never retried as-is, an unavailable authority means
/// nothing was committed and the same call may be retried later, an unknown outcome may already
/// be committed and is only ever retried with the same submission identity, and stale eligibility
/// means the coordination lease must be re-acquired before any further write. A missing record is
/// reported as `None` by reads and as a conflict where a fact was required.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("controller I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("controller storage: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("controller encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("invalid message: {0}")]
    Validation(#[from] MessageValidationError),
    #[error("another runtime owns the Controller database")]
    AlreadyRunning,
    #[error("unrecognized Controller schema or identity")]
    InvalidStorage,
    #[error("input, result or dispatch ownership conflict")]
    Conflict,
    #[error("injected persistence failure")]
    Injected,
    #[error("invalid deployment composition: {0}")]
    Configuration(String),
    /// The authority did not accept the call and committed nothing; retrying later is safe.
    #[error("persistence unavailable: {0}")]
    Unavailable(String),
    /// The reply was lost after the call may have been committed; only the same submission may retry.
    #[error("persistence outcome unknown: {0}")]
    Unknown(String),
    /// The coordination lease this Controller wrote under is no longer current.
    #[error("coordination eligibility is stale; re-acquire the lease before continuing")]
    StaleEligibility,
}

/// Test seams refuse writes before transactions commit, using the same real SQLite and reconciliation.
/// Guards travel with the store onto the blocking pool, hence the thread-safety bounds.
pub trait WriteGuard: Send + 'static {
    /// Prevents a durable boundary; callers must not dispatch or acknowledge on failure.
    fn before_write(&self, point: WritePoint) -> Result<(), Error>;
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WritePoint {
    Accept,
    Takeover,
    Receipt,
    /// Final takeover boundary inside the open transaction, before SQLite commit or any Ack.
    Commit,
}
pub struct DurableWrites;
impl WriteGuard for DurableWrites {
    /// Production delegates all durability failures to SQLite.
    fn before_write(&self, _point: WritePoint) -> Result<(), Error> {
        Ok(())
    }
}

/// An accepted operation and its durable terminal fact as the local catalogue keeps it, at full wire
/// fidelity; no result means awaiting reconciliation, not failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloneOperation {
    pub command: CloneRepositoryMessage,
    pub result: Option<CloneExecutionResult>,
}
