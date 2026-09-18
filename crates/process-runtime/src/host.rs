mod server;
mod worker;
pub use server::serve_process_host;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ora_process_client::GuardianRuns;
use ora_process_protocol::{
    GuardianHostSession, HostCoordination, HostRunIntent, HostRunView, HostScopeView, RunId,
    ScopeId,
};
use tokio::task::JoinSet;

use crate::{HostState, ProcessStateError};

/// Owns durable host intent and bounded, independent Scope exchanges. Call tick while serving clients.
/// Network tasks never own the host journal or its lock; dropping this coordinator aborts those tasks.
pub struct HostCoordinator {
    jobs: JoinSet<(ScopeId, worker::Observation)>,
    active: BTreeSet<ScopeId>,
    progress: BTreeMap<ScopeId, Progress>,
    state: HostState,
    guardian: PathBuf,
    owner: u32,
    cursor: Option<ScopeId>,
}

struct Progress {
    status: HostCoordination,
    retry_at: Instant,
    failures: u32,
}

impl HostCoordinator {
    pub fn new(state: HostState, guardian: PathBuf) -> Self {
        // SAFETY: geteuid only reads the current process identity.
        let owner = unsafe { libc::geteuid() };
        Self {
            jobs: JoinSet::new(),
            active: BTreeSet::new(),
            progress: BTreeMap::new(),
            state,
            guardian,
            owner,
            cursor: None,
        }
    }

    /// Reports the incarnation owned under the original host lock, never a caller-selected epoch.
    pub fn binding(&self) -> ora_process_protocol::HostBinding {
        self.state.binding()
    }

    /// Accepts creation without waiting for guardian startup or making cancellation connection-owned.
    pub fn create_scope(&mut self, scope: ScopeId) -> Result<HostScopeView, ProcessStateError> {
        self.state.record_scope_intent(scope)?;
        self.query_scope(scope)
    }

    /// Persists original parameters before asynchronous dispatch; duplicates retain their identity.
    pub fn start(&mut self, intent: HostRunIntent) -> Result<HostRunView, ProcessStateError> {
        let intent = self.state.record_run_intent(intent)?;
        self.query_run(intent.run)
    }

    /// Accepts force-stop durably even when an independent exchange is currently in flight.
    pub fn stop(&mut self, run: RunId) -> Result<HostRunView, ProcessStateError> {
        self.state.request_run_stop(run)?;
        self.query_run(run)
    }

    /// Seals admission synchronously; a previously accepted in-flight Start remains covered by close.
    pub fn close(&mut self, scope: ScopeId) -> Result<HostScopeView, ProcessStateError> {
        self.state.request_scope_close(scope)?;
        self.query_scope(scope)
    }

    /// Keeps historical facts visible without describing them as fresh observations after recovery.
    pub fn query_run(&self, run: RunId) -> Result<HostRunView, ProcessStateError> {
        let intent = self
            .state
            .run_intent(run)?
            .ok_or(ProcessStateError::Rejected("unknown host Run"))?;
        Ok(HostRunView {
            scope: intent.scope,
            run,
            stop_requested: self.state.run_stop_requested(run)?,
            last_observed: self.state.observed_run(run)?,
            coordination: self
                .progress
                .get(&intent.scope)
                .map_or(HostCoordination::Pending, |progress| {
                    progress.status.clone()
                }),
        })
    }

    /// An accepted close is not complete until a separately recorded observation says so.
    pub fn query_scope(&self, scope: ScopeId) -> Result<HostScopeView, ProcessStateError> {
        self.state
            .scope_intent(scope)?
            .ok_or(ProcessStateError::Rejected("unknown host Scope"))?;
        Ok(HostScopeView {
            scope,
            close_requested: self.state.scope_close_requested(scope)?,
            last_observed: self.state.observed_scope(scope)?,
            coordination: self
                .progress
                .get(&scope)
                .map_or(HostCoordination::Pending, |progress| {
                    progress.status.clone()
                }),
        })
    }

    /// Prepares only a read-only output exchange; its await must happen outside the coordinator lock.
    pub fn output_client(&self, run: RunId) -> Result<GuardianRuns, ProcessStateError> {
        let intent = self
            .state
            .run_intent(run)?
            .ok_or(ProcessStateError::Rejected("unknown host Run"))?;
        let access = self
            .state
            .guardian_access(intent.scope)?
            .ok_or(ProcessStateError::Rejected("guardian not launched"))?;
        Ok(GuardianRuns::new(
            access,
            self.owner,
            GuardianHostSession {
                host: self.binding(),
            },
        ))
    }

    /// Publishes committed observations and schedules one exchange per Scope without awaiting sockets.
    /// The worker cap bounds transport concurrency, not the number of accepted Runs or Scopes.
    pub fn tick(&mut self) -> Result<(), ProcessStateError> {
        while let Some(result) = self.jobs.try_join_next() {
            let (scope, observation) = result
                .map_err(|_| ProcessStateError::Rejected("host coordination worker failed"))?;
            self.active.remove(&scope);
            for snapshot in observation.runs {
                self.state.observe_run(&snapshot)?;
            }
            if let Some(state) = observation.scope {
                self.state.observe_scope(scope, state)?;
            }
            let progress = self.progress.entry(scope).or_insert(Progress {
                status: HostCoordination::Pending,
                retry_at: Instant::now(),
                failures: 0,
            });
            progress.failures = if observation.status == HostCoordination::Observing {
                0
            } else {
                progress.failures.saturating_add(1).min(6)
            };
            // Stagger Scope retries without inventing an attempt limit or abandoning responsibility.
            let jitter = scope.to_string().bytes().map(u64::from).sum::<u64>() % 100;
            progress.retry_at = Instant::now()
                + Duration::from_millis((100_u64 << progress.failures).min(5000) + jitter);
            progress.status = observation.status;
        }
        let scopes = self.state.scope_intents()?;
        let mut ordered: Vec<_> = scopes.into_iter().map(|intent| intent.scope).collect();
        if let Some(cursor) = self.cursor {
            let pivot = ordered.partition_point(|scope| *scope <= cursor);
            ordered.rotate_left(pivot);
        }
        for scope in ordered {
            if self.active.len() >= 32 {
                break;
            }
            if self.active.contains(&scope)
                || self
                    .progress
                    .get(&scope)
                    .is_some_and(|progress| progress.retry_at > Instant::now())
            {
                continue;
            }
            let work = worker::prepare(&mut self.state, scope, &self.guardian, self.owner)?;
            self.jobs
                .spawn(async move { (scope, work.observe().await) });
            self.active.insert(scope);
            self.cursor = Some(scope);
        }
        Ok(())
    }
}
