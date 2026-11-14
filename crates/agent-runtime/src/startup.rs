//! Constructs the runtime manager only after reconciling durable session lifecycle state.
//!
//! Startup recovery belongs to the manager, before any connection or session actor can observe
//! persisted rows. It does not restore provider sessions, which remain attached on demand.

use crate::clock::SystemClock;
use crate::connection::ConnectionSupervisors;
use crate::host::SessionStore;
use crate::{
    AgentRuntimeHost, AgentRuntimeManager, AgentRuntimeSetup, AgentSessionBarriers, ManagerInner,
    RuntimeError,
};
use ora_domain::SessionStatus;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};

impl<H: AgentRuntimeHost> AgentRuntimeManager<H> {
    /// Builds the manager, reconciles stale rows, and immediately starts the shared supervisor.
    pub fn new(setup: AgentRuntimeSetup<H>) -> Result<Self, RuntimeError> {
        let AgentRuntimeSetup {
            attach,
            store,
            directory,
            session_setup: session_mcp,
            events,
            home_directory,
            sessions_root,
            scheduler,
        } = setup;
        let clock = SystemClock;
        reconcile_running_sessions(&store, clock)?;
        let barriers = Arc::new(AgentSessionBarriers::new());
        let connections =
            ConnectionSupervisors::start(attach, store.clone(), home_directory, clock);
        Ok(Self {
            inner: Arc::new(ManagerInner {
                store,
                directory,
                actors: RwLock::new(HashMap::new()),
                unpublished_workflow_sessions: RwLock::new(HashSet::new()),
                lifecycle: tokio::sync::Mutex::new(()),
                next_operation_id: AtomicU64::new(1),
                connections,
                sessions_root,
                session_mcp,
                barriers,
                clock,
                scheduler,
                events,
                #[cfg(feature = "test-support")]
                prompt_inactivity_windows: RwLock::new(None),
            }),
        })
    }
}

/// Restores durable lifecycle truth before the managed connection starts.
fn reconcile_running_sessions(
    repository: &impl SessionStore,
    clock: SystemClock,
) -> Result<(), RuntimeError> {
    for session in repository
        .list_sessions()
        .map_err(|source| RuntimeError::internal("failed to reconcile sessions", source))?
    {
        if session.status == SessionStatus::Running {
            repository
                .update_session_status(
                    &session.id,
                    SessionStatus::Stopped,
                    clock.now_timestamp_millis(),
                )
                .map_err(|source| RuntimeError::internal("failed to reconcile sessions", source))?;
        }
    }
    Ok(())
}
