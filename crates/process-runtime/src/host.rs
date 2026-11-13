#[cfg(test)]
mod non_launch_tests;
#[cfg(test)]
mod scope_cache_tests;
mod server;
mod worker;
pub use server::serve_process_host;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ora_process_client::GuardianRuns;
use ora_process_protocol::{
    GuardianHostSession, GuardianRunRejection, HostCoordination, HostRunIntent, HostRunView,
    HostScopeView, RunId, ScopeId,
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
    // Mirrors the journal's Scope intents so tick does not re-read the whole table every pass.
    // Only this coordinator inserts them and nothing deletes them, so loading once and appending
    // after each committed insert keeps this equal to the journal.
    scopes: BTreeSet<ScopeId>,
    guardian: PathBuf,
    owner: u32,
    cursor: Option<ScopeId>,
}

struct Progress {
    status: HostCoordination,
    retry_at: Instant,
    failures: u32,
    control_pending: bool,
    // Derived only after this host has reconciled the original guardian and persisted all facts.
    // Recovery starts without this cache, so old output authority is rebound before settling again.
    settled: bool,
}

impl HostCoordinator {
    /// Loads the accepted Scopes once, so created and recovered state start from the same journal.
    pub fn new(state: HostState, guardian: PathBuf) -> Result<Self, ProcessStateError> {
        // SAFETY: geteuid only reads the current process identity.
        let owner = unsafe { libc::geteuid() };
        let scopes = state
            .scope_intents()?
            .into_iter()
            .map(|intent| intent.scope)
            .collect();
        Ok(Self {
            jobs: JoinSet::new(),
            active: BTreeSet::new(),
            progress: BTreeMap::new(),
            state,
            scopes,
            guardian,
            owner,
            cursor: None,
        })
    }

    /// Reports the incarnation owned under the original host lock, never a caller-selected epoch.
    pub fn binding(&self) -> ora_process_protocol::HostBinding {
        self.state.binding()
    }

    /// Accepts creation without waiting for guardian startup or making cancellation connection-owned.
    pub fn create_scope(&mut self, scope: ScopeId) -> Result<HostScopeView, ProcessStateError> {
        // Cache only what the journal accepted; a failed insert must not become a tracked Scope.
        self.state.record_scope_intent(scope)?;
        self.scopes.insert(scope);
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
        let already_requested = self.state.scope_close_requested(scope)?;
        self.state.request_scope_close(scope)?;
        // Observation backoff must not defer newly accepted control intent. An in-flight
        // exchange still owns its Scope; tick picks up the durable close on its next pass.
        if !already_requested {
            let progress = self.progress.entry(scope).or_insert(Progress {
                status: HostCoordination::Pending,
                retry_at: Instant::now(),
                failures: 0,
                control_pending: false,
                settled: false,
            });
            progress.retry_at = Instant::now();
            progress.control_pending = true;
        }
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
            let mut status = observation.status;
            for snapshot in observation.runs {
                match self.state.observe_run(&snapshot) {
                    Ok(()) => {}
                    // The journal retains the original fact. Reject only this snapshot, expose
                    // the conflict and keep persisting its peers; storage failures still stop tick.
                    Err(ProcessStateError::ContradictoryRunObservation(_)) => {
                        status = HostCoordination::Rejected(GuardianRunRejection::ConflictingRun);
                    }
                    Err(error) => return Err(error),
                }
            }
            if let Some(state) = observation.scope {
                self.state.observe_scope(scope, state)?;
            }
            let progress = self.progress.entry(scope).or_insert(Progress {
                status: HostCoordination::Pending,
                retry_at: Instant::now(),
                failures: 0,
                control_pending: false,
                settled: false,
            });
            progress.failures = if status == HostCoordination::Observing {
                0
            } else {
                progress.failures.saturating_add(1).min(6)
            };
            // Stagger Scope retries without inventing an attempt limit or abandoning responsibility.
            let jitter = scope.to_string().bytes().map(u64::from).sum::<u64>() % 100;
            progress.retry_at = if progress.control_pending {
                Instant::now()
            } else {
                Instant::now()
                    + Duration::from_millis((100_u64 << progress.failures).min(5000) + jitter)
            };
            progress.control_pending = false;
            progress.status = status;
        }
        // Resume after the last dispatched Scope and wrap around, so a full worker cap rotates fairly.
        let resume = self.cursor.map_or(Bound::Unbounded, Bound::Excluded);
        let wrapped = self
            .cursor
            .into_iter()
            .flat_map(|cursor| self.scopes.range(..=cursor));
        for &scope in self.scopes.range((resume, Bound::Unbounded)).chain(wrapped) {
            if self.active.len() >= 32 {
                break;
            }
            if self.active.contains(&scope)
                || self
                    .progress
                    .get(&scope)
                    .is_some_and(|progress| progress.settled || progress.retry_at > Instant::now())
            {
                continue;
            }
            if let Some(progress) = self.progress.get_mut(&scope)
                && progress.status == HostCoordination::Observing
                && self.state.scope_settled(scope)?
            {
                progress.settled = true;
                continue;
            }
            let work = worker::prepare(&mut self.state, scope, &self.guardian, self.owner)?;
            self.jobs
                .spawn(async move { (scope, work.observe().await) });
            self.active.insert(scope);
            if let Some(progress) = self.progress.get_mut(&scope) {
                progress.control_pending = false;
            }
            self.cursor = Some(scope);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;

    /// Confirmed terminal Scopes must not consume all exchange slots when their guardians stop replying.
    #[tokio::test]
    async fn closed_scope_guardians_cannot_block_a_fresh_scope()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
        let directory = tempfile::Builder::new()
            .prefix("hc")
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(home)?;
        let guardian = directory.path().join("guardian");
        std::fs::copy("/usr/bin/true", &guardian)?;
        std::fs::set_permissions(&guardian, std::fs::Permissions::from_mode(/*mode*/ 0o700))?;
        let mut host = HostCoordinator::new(
            HostState::create(&directory.path().join("s"))?,
            guardian.clone(),
        )?;
        let mut listeners = Vec::new();
        for index in 1..=32 {
            let scope: ScopeId = format!("00000000-0000-0000-0000-{index:012x}").parse()?;
            host.create_scope(scope)?;
            // The real launch boundary supplies the original access identity. The fixture has
            // already observed closure; its old listener now accepts but never answers again.
            drop(host.state.begin_guardian(scope, &guardian)?);
            let access = host.state.guardian_access(scope)?.ok_or("missing access")?;
            host.close(scope)?;
            host.state.observe_scope(
                scope,
                ora_process_protocol::ScopeState::Closed(
                    ora_process_protocol::CleanupEvidence::BestEffortComplete,
                ),
            )?;
            host.progress.insert(
                scope,
                Progress {
                    status: HostCoordination::Observing,
                    retry_at: Instant::now(),
                    failures: 0,
                    control_pending: false,
                    settled: false,
                },
            );
            listeners.push(tokio::net::UnixListener::bind(
                access
                    .scope_dir
                    .join(ora_process_protocol::GuardianChannel::Control.socket_name()),
            )?);
        }
        let fresh: ScopeId = "ffffffff-ffff-ffff-ffff-ffffffffffff".parse()?;
        host.create_scope(fresh)?;
        let result = tokio::time::timeout(Duration::from_secs(/*secs*/ 2), async {
            loop {
                host.tick()?;
                if host.state.guardian_access(fresh)?.is_some() {
                    return Ok::<_, ProcessStateError>(());
                }
                tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
            }
        })
        .await;
        assert!(
            result.is_ok(),
            "confirmed closed guardians occupied all slots and prevented fresh Scope dispatch"
        );
        result??;
        drop(listeners);
        Ok(())
    }

    /// A previously unavailable guardian must not delay newly accepted close intent behind backoff.
    #[tokio::test]
    async fn close_dispatches_without_waiting_for_observation_backoff()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
        let directory = tempfile::Builder::new()
            .prefix("hc")
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(home)?;
        let state = HostState::create(&directory.path().join("s"))?;
        let mut host = HostCoordinator::new(state, directory.path().join("unused"))?;
        let scope = ScopeId::new();
        host.create_scope(scope)?;
        host.progress.insert(
            scope,
            Progress {
                status: HostCoordination::Unavailable,
                retry_at: Instant::now() + Duration::from_secs(/*secs*/ 6),
                failures: 6,
                control_pending: false,
                settled: false,
            },
        );
        host.close(scope)?;
        host.tick()?;
        assert!(
            host.active.contains(&scope),
            "accepted close is still waiting behind observation backoff"
        );
        Ok(())
    }

    /// A repeated close is observation traffic, not permission to defeat failure backoff.
    #[tokio::test]
    async fn duplicate_close_preserves_retry_backoff() -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
        let directory = tempfile::Builder::new()
            .prefix("hc")
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(home)?;
        let mut host = HostCoordinator::new(
            HostState::create(&directory.path().join("s"))?,
            directory.path().join("unused"),
        )?;
        let scope = ScopeId::new();
        host.create_scope(scope)?;
        host.close(scope)?;
        let retry_at = Instant::now() + Duration::from_secs(/*secs*/ 6);
        host.progress.insert(
            scope,
            Progress {
                status: HostCoordination::Unavailable,
                retry_at,
                failures: 6,
                control_pending: false,
                settled: false,
            },
        );
        host.close(scope)?;
        assert_eq!(host.progress[&scope].retry_at, retry_at);
        host.tick()?;
        assert!(!host.active.contains(&scope));
        Ok(())
    }

    /// A close accepted during the first exchange survives its stale unavailable observation.
    #[tokio::test]
    async fn close_during_first_exchange_is_dispatched_after_that_exchange()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
        let directory = tempfile::Builder::new()
            .prefix("hc")
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(home)?;
        let mut host = HostCoordinator::new(
            HostState::create(&directory.path().join("s"))?,
            directory.path().join("unused"),
        )?;
        let scope = ScopeId::new();
        host.create_scope(scope)?;
        host.active.insert(scope);
        host.jobs.spawn(async move {
            (
                scope,
                worker::Observation {
                    scope: None,
                    runs: vec![],
                    status: HostCoordination::Unavailable,
                },
            )
        });
        host.close(scope)?;
        assert!(host.active.contains(&scope));
        tokio::task::yield_now().await;
        host.tick()?;
        assert!(
            host.active.contains(&scope),
            "stale exchange put the accepted close back into failure backoff"
        );
        Ok(())
    }
}
