//! Drives durable Effect reconcile requests until each declared surface matches Desired State.
//!
//! The worker owns no state of its own. Every pass re-reads what the database currently says is
//! owed, which is what makes a lost in-process wakeup harmless: the periodic scan finds the same
//! request again, and a request that was merged with a later edit is served once at the newer
//! generation rather than replayed per edit.

use crate::agent_runtime::{ReplacedAgentSessions, plugin_agent};
use crate::clock::SystemClock;
use crate::effect_surface_registration::converge_workspace_surfaces;
use crate::plugin::PluginApi;
use ora_application::Clock;
use ora_db::{
    ClaimedReconcile, DueSurfaceReconcile, ReconcileClaim, RepositoryPool, SqliteEffectRepository,
    SqliteWorkspaceRepository,
};
use ora_domain::PluginId;
use ora_effect::{
    Condition, ConsumerCoordinator, ConsumerId, CoordinationError, CoordinationOutcome,
    FilesystemSurfaceAdapter, Generation, Reconciler, RetryPolicy, SurfaceKey, SurfaceLifecycle,
    SurfacePath, UuidManagedIdentityGenerator,
};
use ora_logging::{ora_info, ora_warn};
use std::cell::Cell;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;
use tokio::runtime::Handle;
use uuid::Uuid;

/// Idle interval between scans when nothing wakes the worker.
const SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// Upper bound on how many surfaces one pass reconciles, for fairness across Workspaces.
const SURFACE_BATCH_SIZE: usize = 16;
/// How long a claim stays valid without renewal.
///
/// Long enough that an ordinary reconcile never has to renew, short enough that a crashed worker's
/// surfaces become claimable again well inside a user's patience.
const LEASE_DURATION: Duration = Duration::from_secs(60);
/// How often a claim is renewed while one reconcile is still running.
const LEASE_RENEWAL_INTERVAL: Duration = Duration::from_secs(20);
/// How often blocked requests are re-armed, covering runtime events lost to a crash.
const SAFETY_SCAN_INTERVAL: Duration = Duration::from_secs(300);
/// Backoff before the next attempt, indexed by the attempt that just failed.
const RETRY_BACKOFF_MS: [i64; 5] = [5_000, 30_000, 120_000, 600_000, 1_800_000];

/// Coalesced wake-up signal shared between Desired writers and the worker thread.
///
/// The signal only reduces latency. SQLite holds the durable request, so a lost notification costs
/// at most one scan interval and never a reconcile.
#[derive(Debug, Default)]
struct WakeSignal {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl WakeSignal {
    /// Requests one worker pass; concurrent requests coalesce into the same pass.
    fn notify(&self) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_one();
    }

    /// Waits until notified or until the scan interval elapses.
    fn wait(&self, timeout: Duration) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if !*pending {
            let (guard, _timed_out) = self
                .changed
                .wait_timeout(pending, timeout)
                .unwrap_or_else(PoisonError::into_inner);
            pending = guard;
        }
        *pending = false;
    }
}

/// Wakes the Effect worker after a Desired or declaration change is already committed.
#[derive(Clone, Debug)]
pub(crate) struct EffectWorkerHandle {
    wake: Arc<WakeSignal>,
}

impl EffectWorkerHandle {
    /// Asks for one pass without naming a Workspace, generation, or payload.
    ///
    /// Carrying no arguments is deliberate: the worker must re-read current state anyway, so a
    /// caller cannot accidentally pin it to a snapshot that a later commit has already replaced.
    pub(crate) fn notify(&self) {
        self.wake.notify();
    }

    /// Builds a handle with no worker behind it, for APIs assembled without one in tests.
    #[cfg(test)]
    pub(crate) fn unwatched() -> Self {
        Self {
            wake: Arc::new(WakeSignal::default()),
        }
    }

    /// Reports whether a pass is currently owed, so tests can pin the wake without a worker.
    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        *self
            .wake
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Reconciles every surface owing work, coordinating live Agent plugins around each mutation.
pub(crate) struct EffectWorker<Sessions> {
    repository: SqliteEffectRepository,
    workspace_repository: SqliteWorkspaceRepository,
    plugin_host: Arc<PluginApi>,
    /// Repairs the sessions a coordinated restart invalidates.
    sessions: Arc<Sessions>,
    clock: SystemClock,
    wake: Arc<WakeSignal>,
    /// Identifies this worker's claims; a fresh value per process so a crashed one is never
    /// mistaken for the live one when its rows are still marked claimed.
    worker_id: String,
    /// When the low-frequency safety scan may run again.
    next_safety_scan: Mutex<i64>,
}

impl<Sessions: ReplacedAgentSessions> EffectWorker<Sessions> {
    pub(crate) fn new(
        pool: RepositoryPool,
        plugin_host: Arc<PluginApi>,
        sessions: Arc<Sessions>,
    ) -> Self {
        Self {
            repository: SqliteEffectRepository::new(pool.clone()),
            workspace_repository: SqliteWorkspaceRepository::new(pool),
            plugin_host,
            sessions,
            clock: SystemClock,
            wake: Arc::new(WakeSignal::default()),
            worker_id: Uuid::new_v4().to_string(),
            next_safety_scan: Mutex::new(0),
        }
    }

    /// Hands out the wake handle before the worker takes ownership of itself in `spawn`.
    pub(crate) fn handle(&self) -> EffectWorkerHandle {
        EffectWorkerHandle {
            wake: self.wake.clone(),
        }
    }

    /// Runs passes on a dedicated thread until the process ends.
    ///
    /// A plain OS thread rather than a Tokio task, because reconciliation is synchronous
    /// filesystem work that would otherwise occupy an async worker for the whole copy. The thread
    /// owns the small runtime its plugin IPC needs instead of borrowing the caller's, so the
    /// worker does not require `Backend::open` to itself run inside a runtime.
    pub(crate) fn spawn(self) -> EffectWorkerHandle {
        let handle = self.handle();
        let spawned = std::thread::Builder::new()
            .name("effect-worker".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        ora_warn!(
                            operation = "effect_reconcile",
                            error = %error,
                            "failed to build the Effect worker runtime; reconciliation deferred to next start",
                        );
                        return;
                    }
                };
                loop {
                    self.run_pass(runtime.handle());
                    self.wake.wait(SCAN_INTERVAL);
                }
            });
        if let Err(error) = spawned {
            // Without the worker nothing materializes this process lifetime, but every request
            // stays in SQLite; the next start picks them all up.
            ora_warn!(
                operation = "effect_reconcile",
                error = %error,
                "failed to spawn Effect worker thread; reconciliation deferred to next start",
            );
        }
        handle
    }

    /// Rebuilds work a previous process left unscheduled, before serving ordinary wakeups.
    ///
    /// Running this first is what makes a crash recoverable rather than silently lossy: a surface
    /// left short of its generation, an operation left unfinished, or a lease left held by a dead
    /// process all become claimable again here.
    pub(crate) fn recover(&self) {
        match self
            .repository
            .recover_reconcile_requests(self.clock.now_timestamp_millis())
        {
            Ok(0) => {}
            Ok(recovered) => ora_info!(
                operation = "effect_reconcile",
                recovered = recovered,
                "rescheduled Effect work left behind by a previous process",
            ),
            Err(error) => ora_warn!(
                operation = "effect_reconcile",
                error = %error,
                "Effect startup recovery failed; the next safety scan retries it",
            ),
        }
    }

    /// Reconciles one batch of claimed surfaces, isolating each so one cannot stall the rest.
    pub(crate) fn run_pass(&self, runtime: &Handle) {
        let now = self.clock.now_timestamp_millis();
        self.run_safety_scan(now);
        self.converge_surface_registrations(now);
        let claimed = match self.repository.claim_due_reconcile_requests(
            &self.worker_id,
            now,
            now + LEASE_DURATION.as_millis() as i64,
            SURFACE_BATCH_SIZE,
        ) {
            Ok(claimed) => claimed,
            Err(error) => {
                ora_warn!(
                    operation = "effect_reconcile",
                    error = %error,
                    "failed to claim due Effect reconcile requests",
                );
                return;
            }
        };
        for request in claimed {
            self.reconcile_claimed(runtime, request);
        }
    }

    /// Re-arms blocked requests occasionally, covering a runtime event lost to a crash.
    fn run_safety_scan(&self, now: i64) {
        {
            let mut next = self
                .next_safety_scan
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if now < *next {
                return;
            }
            *next = now + SAFETY_SCAN_INTERVAL.as_millis() as i64;
        }
        match self.repository.rearm_blocked_reconcile_requests(now) {
            Ok(0) => {}
            Ok(rearmed) => ora_info!(
                operation = "effect_reconcile",
                rearmed = rearmed,
                "safety scan re-armed blocked Effect surfaces",
            ),
            Err(error) => ora_warn!(
                operation = "effect_reconcile",
                error = %error,
                "Effect safety scan failed",
            ),
        }
    }

    /// Gives Workspaces that no declaration could reach the surfaces the current consumers ask for.
    ///
    /// This runs before claiming so a Workspace registered here is served in the same pass rather
    /// than waiting out another scan interval. A failure is logged rather than propagated: the
    /// next pass re-derives the same set, and the surfaces that are already registered still owe
    /// their reconcile regardless.
    fn converge_surface_registrations(&self, now: i64) {
        let declarations = self.plugin_host.agent_effect_surface_declarations();
        if declarations.is_empty() {
            return;
        }
        let workspaces = match self.workspace_repository.list_all_workspaces() {
            Ok(workspaces) => workspaces,
            Err(error) => {
                ora_warn!(
                    operation = "effect_reconcile",
                    error = %error,
                    "failed to list Workspaces for Effect surface convergence",
                );
                return;
            }
        };
        match converge_workspace_surfaces(&self.repository, &workspaces, &declarations, now) {
            Ok(0) => {}
            Ok(registered) => ora_info!(
                operation = "effect_reconcile",
                registered = registered,
                "registered Effect surfaces for Workspaces created after the last declaration",
            ),
            Err(error) => ora_warn!(
                operation = "effect_reconcile",
                error = %error,
                "failed to register Effect surfaces for a Workspace; the next pass retries",
            ),
        }
    }

    /// Runs one claimed surface against the live Agent plugins declared as its consumers.
    fn reconcile_claimed(&self, runtime: &Handle, request: ClaimedReconcile) {
        let ClaimedReconcile { claim, due } = request;
        let workspace_root = due.workspace_root.clone();
        let relative_path = due.descriptor.path.clone();
        // Coordination can wait on a consumer for as long as a turn runs, so the lease is renewed
        // underneath the reconcile rather than being sized for the slowest possible one.
        let renewal = LeaseRenewal::start(self, &claim);
        let coordinator = PluginSurfaceCoordinator {
            plugin_host: self.plugin_host.as_ref(),
            sessions: self.sessions.as_ref(),
            runtime,
            workspace_root: &workspace_root,
            relative_path: &relative_path,
            quiesced: Cell::new(false),
        };
        let outcome = reconcile_one(
            &self.repository,
            &coordinator,
            due,
            self.clock.now_timestamp_millis(),
        );
        renewal.stop();
        self.settle(&claim, outcome);
    }

    /// Records what the reconcile decided, choosing the schedule its outcome earns.
    fn settle(&self, claim: &ReconcileClaim, outcome: SurfaceOutcome) {
        let now = self.clock.now_timestamp_millis();
        let result = match outcome {
            SurfaceOutcome::Converged { generation } => self
                .repository
                .complete_reconcile_request(claim, generation, now)
                .map(|_| ()),
            // Nothing this worker can do sooner helps, so the surface is parked until an external
            // fact changes rather than burning attempts against an unmet precondition.
            SurfaceOutcome::Blocked { reason } => self
                .repository
                .block_reconcile_request(claim, reason, now)
                .map(|_| ()),
            SurfaceOutcome::Retry { reason } => {
                let delay = backoff_delay(claim.attempt);
                self.repository
                    .retry_reconcile_request(claim, reason, now + delay, now)
                    .map(|_| ())
            }
        };
        if let Err(error) = result {
            // The claim's lease still expires on its own, so a failure to record the decision
            // costs one lease interval rather than the surface.
            ora_warn!(
                operation = "effect_reconcile",
                surface = claim.surface_key.as_str(),
                error = %error,
                "failed to record an Effect reconcile outcome; the lease will expire and retry",
            );
        }
    }
}

/// What one reconcile earned, in the terms the request store schedules on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceOutcome {
    Converged { generation: Generation },
    Blocked { reason: &'static str },
    Retry { reason: &'static str },
}

/// Spreads retries of a shared failure apart instead of stacking every surface on one instant.
///
/// The jitter is derived from a fresh UUID rather than a seeded generator: it only has to break
/// synchronization between surfaces, and nothing depends on the sequence being reproducible.
fn backoff_delay(attempt: i64) -> i64 {
    let index = (attempt.max(1) - 1).clamp(0, RETRY_BACKOFF_MS.len() as i64 - 1) as usize;
    let base = RETRY_BACKOFF_MS[index];
    let spread = base / 4;
    let jitter = i64::from(Uuid::new_v4().as_bytes()[0]) * spread / i64::from(u8::MAX);
    base - spread / 2 + jitter
}

/// Keeps one claim alive on a background thread for as long as its reconcile runs.
struct LeaseRenewal {
    stop: Arc<(Mutex<bool>, Condvar)>,
    joiner: Option<std::thread::JoinHandle<()>>,
}

impl LeaseRenewal {
    /// Starts renewing until `stop`, leaving the claim untouched if the thread cannot start.
    fn start<Sessions>(worker: &EffectWorker<Sessions>, claim: &ReconcileClaim) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let repository = worker.repository.clone();
        let worker_id = worker.worker_id.clone();
        let claim = claim.clone();
        let clock = worker.clock;
        let signal = stop.clone();
        let joiner = std::thread::Builder::new()
            .name("effect-lease".to_string())
            .spawn(move || {
                let (lock, changed) = &*signal;
                loop {
                    let mut stopped = lock.lock().unwrap_or_else(PoisonError::into_inner);
                    if !*stopped {
                        let (guard, _timed_out) = changed
                            .wait_timeout(stopped, LEASE_RENEWAL_INTERVAL)
                            .unwrap_or_else(PoisonError::into_inner);
                        stopped = guard;
                    }
                    if *stopped {
                        return;
                    }
                    drop(stopped);
                    let now = clock.now_timestamp_millis();
                    match repository.renew_reconcile_claim(
                        &claim,
                        &worker_id,
                        now + LEASE_DURATION.as_millis() as i64,
                        now,
                    ) {
                        // Losing the lease means another worker already owns this surface. There is
                        // nothing safe left to renew, so renewal simply stops; the in-flight
                        // reconcile's own writes are fenced by the token it no longer holds.
                        Ok(false) => return,
                        Ok(true) => {}
                        Err(error) => ora_warn!(
                            operation = "effect_reconcile",
                            surface = claim.surface_key.as_str(),
                            error = %error,
                            "failed to renew an Effect reconcile lease",
                        ),
                    }
                }
            })
            .ok();
        Self { stop, joiner }
    }

    /// Ends renewal and waits for the thread, so no renewal outlives the work it protected.
    fn stop(mut self) {
        let (lock, changed) = &*self.stop;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        changed.notify_all();
        if let Some(joiner) = self.joiner.take() {
            let _ = joiner.join();
        }
    }
}

/// Runs one surface through scan, plan, coordinated mutation, and durable status.
///
/// Returns what the surface earned rather than scheduling it, so the request-store transitions stay
/// with the claim that authorizes them, and so the whole decision can be exercised against a
/// substituted coordinator.
fn reconcile_one<Coordinator: ConsumerCoordinator>(
    repository: &SqliteEffectRepository,
    coordinator: &Coordinator,
    due: DueSurfaceReconcile,
    occurred_at: i64,
) -> SurfaceOutcome {
    let adapter = FilesystemSurfaceAdapter::new(
        due.workspace_id.clone(),
        due.workspace_root.clone(),
        due.descriptor.surface_key.clone(),
        due.descriptor.path.clone(),
    );
    let identity_generator = UuidManagedIdentityGenerator;
    let reconciler = Reconciler::new(repository, repository, coordinator, &identity_generator);

    let outcome = match reconciler.reconcile_surface(
        &adapter,
        &due.descriptor,
        &due.workspace_id,
        occurred_at,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            // A scan or filesystem failure is exactly what a timed retry is for: nothing about the
            // declaration is wrong, the target was momentarily unreadable.
            ora_warn!(
                operation = "effect_reconcile",
                surface = due.descriptor.surface_key.as_str(),
                error = %error,
                "Effect surface reconcile failed; scheduling a backoff retry",
            );
            return SurfaceOutcome::Retry {
                reason: "reconcile_failed",
            };
        }
    };

    ora_info!(
        operation = "effect_reconcile",
        surface = due.descriptor.surface_key.as_str(),
        phase = ?outcome.status.phase,
        applied_generation = outcome.status.applied_generation.value(),
        desired_generation = outcome.status.desired_generation.value(),
        "reconciled one Effect surface",
    );

    // A condition is the reconciler's own account of why it could not finish, and its reason
    // already carries the retry policy that reason deserves; deriving the schedule from the policy
    // keeps that judgement in the domain instead of re-deciding it per call site here.
    if let Some(condition) = strictest_condition(&outcome.status.conditions) {
        return match condition.retry_policy {
            RetryPolicy::Manual => SurfaceOutcome::Blocked {
                reason: "recovery_required",
            },
            RetryPolicy::Backoff => SurfaceOutcome::Retry {
                reason: "transient_failure",
            },
            RetryPolicy::OnChange => SurfaceOutcome::Blocked {
                reason: "awaiting_external_change",
            },
        };
    }

    // Only a surface whose files were confirmed to match may clear its request; anything short of
    // that stays owed so a later pass re-reads it rather than treating partial work as done.
    if outcome.status.applied_generation < outcome.status.desired_generation {
        return SurfaceOutcome::Retry {
            reason: "generation_not_applied",
        };
    }
    let generation = outcome.status.applied_generation;
    finish_retirement(repository, &due, generation);
    SurfaceOutcome::Converged { generation }
}

/// Picks the condition whose policy decides the schedule, strictest first.
///
/// Manual outranks everything because an unproven target must never be retried automatically, and a
/// timed backoff outranks waiting on an external change so a transient failure still makes progress
/// when both are present.
fn strictest_condition(conditions: &[Condition]) -> Option<&Condition> {
    let rank = |condition: &Condition| match condition.retry_policy {
        RetryPolicy::Manual => 0,
        RetryPolicy::Backoff => 1,
        RetryPolicy::OnChange => 2,
    };
    conditions.iter().min_by_key(|condition| rank(condition))
}

/// Deletes a retired surface once its ledger is empty, ending the lifecycle Ora started.
fn finish_retirement(
    repository: &SqliteEffectRepository,
    due: &DueSurfaceReconcile,
    completed: Generation,
) {
    if due.descriptor.lifecycle != SurfaceLifecycle::Retiring {
        return;
    }
    match repository.delete_retired_surface(&due.descriptor.surface_key) {
        Ok(true) => ora_info!(
            operation = "effect_reconcile",
            surface = due.descriptor.surface_key.as_str(),
            generation = completed.value(),
            "retired Effect surface removed after its ledger was emptied",
        ),
        // Still-owned targets keep the surface alive on purpose; the ledger outlives the
        // declaration until cleanup can prove every managed target is gone.
        Ok(false) => {}
        Err(error) => ora_warn!(
            operation = "effect_reconcile",
            surface = due.descriptor.surface_key.as_str(),
            error = %error,
            "failed to delete a retired Effect surface",
        ),
    }
}

/// Bridges the synchronous reconciler onto one surface's live Agent plugin consumers.
///
/// The coordination contract is per-surface while the port is per-consumer, so the locator travels
/// on the struct rather than through the trait: Ora resolves and validates the absolute Workspace
/// root, and a plugin only ever receives the path it already declared.
struct PluginSurfaceCoordinator<'a, Sessions> {
    plugin_host: &'a PluginApi,
    sessions: &'a Sessions,
    runtime: &'a Handle,
    workspace_root: &'a Path,
    relative_path: &'a SurfacePath,
    /// Whether this reconcile actually barriered the consumers before mutating the surface.
    ///
    /// Only a barriered reconcile is about to change files under a live agent, which is what makes
    /// the plugin replace its process; resuming a surface that was already current must not cost
    /// the user their sessions.
    quiesced: Cell<bool>,
}

impl<Sessions> PluginSurfaceCoordinator<'_, Sessions> {
    /// Resolves one consumer onto the running plugin generation that must be coordinated.
    ///
    /// A consumer whose plugin is not currently running needs no coordination at all: it holds no
    /// turn that a mutation could corrupt, and it re-reads the surface when it next starts. Only a
    /// live generation can be asked to quiesce, so absence resolves to `None` rather than an error
    /// that would block materialization whenever the agent happens to be disconnected.
    fn running_runtime(&self, plugin_id: &PluginId) -> Option<ora_plugin_runtime::PluginRuntime> {
        self.plugin_host
            .lifecycle
            .connection(plugin_id)
            .ok()
            .map(|connection| connection.runtime().process().clone())
    }
}

impl<Sessions: ReplacedAgentSessions> ConsumerCoordinator
    for PluginSurfaceCoordinator<'_, Sessions>
{
    /// Asks every live consumer to reach an idle boundary, stopping at the first one still busy.
    ///
    /// Reporting `WaitingForIdle` as soon as one consumer is busy is what keeps the barrier
    /// idempotent: consumers already holding theirs keep holding it, and the next pass re-asks
    /// everyone rather than tracking who answered on a previous attempt.
    fn quiesce(
        &self,
        surface_key: &SurfaceKey,
        consumers: &[ConsumerId],
    ) -> Result<CoordinationOutcome, CoordinationError> {
        for consumer in consumers {
            let plugin_id = PluginId::parse(consumer.as_str()).map_err(CoordinationError::new)?;
            let Some(runtime) = self.running_runtime(&plugin_id) else {
                continue;
            };
            let outcome = self
                .runtime
                .block_on(plugin_agent::wait_for_idle(
                    &runtime,
                    surface_key,
                    self.workspace_root,
                    self.relative_path,
                ))
                .map_err(CoordinationError::new)?;
            if outcome == plugin_agent::WaitForIdleOutcome::WaitingForIdle {
                return Ok(CoordinationOutcome::WaitingForIdle);
            }
        }
        self.quiesced.set(true);
        Ok(CoordinationOutcome::Ready)
    }

    /// Restarts one consumer so it observes the generation just written, releasing its barrier.
    ///
    /// A restart that followed a barrier replaced the agent's process, and every provider-side
    /// session that process held died with it. Ora still holds those session ids, so the sessions
    /// are detached here rather than left to fail their next prompt against an agent that has never
    /// heard of them. Only the barriered case detaches: a surface that was already current resumes
    /// without the plugin replacing anything, and stopping live sessions for that would cost the
    /// user a conversation to repair nothing.
    fn resume(
        &self,
        surface_key: &SurfaceKey,
        consumer: &ConsumerId,
        generation: Generation,
    ) -> Result<(), CoordinationError> {
        let plugin_id = PluginId::parse(consumer.as_str()).map_err(CoordinationError::new)?;
        let Some(runtime) = self.running_runtime(&plugin_id) else {
            return Ok(());
        };
        self.runtime
            .block_on(plugin_agent::restart(
                &runtime,
                surface_key,
                self.workspace_root,
                self.relative_path,
                generation,
            ))
            .map_err(CoordinationError::new)?;
        if self.quiesced.get() {
            self.sessions
                .detach_sessions_for_replaced_plugin(&plugin_id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EffectWorker, EffectWorkerHandle, ReplacedAgentSessions, SurfaceOutcome, reconcile_one,
    };
    use crate::app_event::AppEventHub;
    use crate::effect_surface_registration::converge_workspace_surfaces;
    use crate::plugin::PluginApi;
    use crate::project::ProjectApi;
    use crate::user_config::UserConfigApi;
    use ora_application::Clock;
    use ora_contracts::CreateProjectRequest;
    use ora_db::{
        ClaimedReconcile, DatabaseBootstrapper, DatabaseLocation, RepositoryPool,
        SourcePublication, SqliteEffectRepository, SqliteWorkspaceRepository,
        default_migration_catalog,
    };
    use ora_domain::{Namespace, PluginId, WorkspaceId};
    use ora_effect::{
        ConsumerCoordination, ConsumerCoordinator, ConsumerId, CoordinationError,
        CoordinationOutcome, DesiredSkillState, Digest, EffectRepository, FilesystemSkillSurface,
        Generation, MARKER_FILE_NAME, MaterializationFormat, SkillName, SkillSelectionKey,
        SkillSource, SkillState, SourceKind, SourceVersion, SurfaceDescriptorSet, SurfaceKey,
        SurfacePath, WorkspaceEffectSpec,
    };
    use pretty_assertions::assert_eq;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, PoisonError};
    use tempfile::TempDir;

    /// Later than any row the real clock writes during `ProjectApi::create`, whose Workspace
    /// trigger seeds `workspace_effects` and whose CHECK forbids a write dated before it.
    const PUBLISHED_AT: i64 = 4_000_000_000_000;
    const WORKER: &str = "worker-1";

    const MANIFEST: &str = "---\nname: grilling\ndescription: Grill a plan relentlessly.\n---\n\nAsk hard questions.\n";

    /// Records which agents were detached after a coordinated restart.
    #[derive(Debug, Default)]
    struct RecordingSessions {
        detached: Mutex<Vec<String>>,
    }

    impl ReplacedAgentSessions for RecordingSessions {
        fn detach_sessions_for_replaced_plugin(&self, plugin_id: &PluginId) {
            self.detached
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(plugin_id.canonical());
        }
    }

    /// Records coordination calls and answers with a scripted quiesce outcome.
    #[derive(Debug, Default)]
    struct RecordingCoordinator {
        busy: bool,
        calls: Mutex<Vec<String>>,
    }

    impl ConsumerCoordinator for RecordingCoordinator {
        fn quiesce(
            &self,
            _surface_key: &SurfaceKey,
            consumers: &[ConsumerId],
        ) -> Result<CoordinationOutcome, CoordinationError> {
            for consumer in consumers {
                self.calls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(format!("quiesce:{}", consumer.as_str()));
            }
            Ok(if self.busy {
                CoordinationOutcome::WaitingForIdle
            } else {
                CoordinationOutcome::Ready
            })
        }

        fn resume(
            &self,
            _surface_key: &SurfaceKey,
            consumer: &ConsumerId,
            generation: Generation,
        ) -> Result<(), CoordinationError> {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!(
                    "resume:{}@{}",
                    consumer.as_str(),
                    generation.value()
                ));
            Ok(())
        }
    }

    /// Builds a pool whose single Workspace is the given directory, via the real create path.
    ///
    /// Going through `ProjectApi` instead of inserting rows keeps the fixture honest about what a
    /// Workspace is, including the location row the Effect surface locator is resolved against.
    fn fixture(data_root: &Path, workspace_root: &Path) -> (RepositoryPool, WorkspaceId) {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(data_root.join("ora.sqlite3")),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        let workspace_id = create_project_workspace(&pool, data_root, workspace_root, "Demo");
        (pool, workspace_id)
    }

    /// Creates one more Project and returns the Workspace it owns, through the real create path.
    ///
    /// Separate from `fixture` so a Workspace can also appear *after* the system already holds
    /// state, which is the ordering a running Ora produces every time a Project or Task is added.
    fn create_project_workspace(
        pool: &RepositoryPool,
        data_root: &Path,
        workspace_root: &Path,
        name: &str,
    ) -> WorkspaceId {
        fs::create_dir_all(workspace_root).unwrap();
        let existing = SqliteWorkspaceRepository::new(pool.clone())
            .list_all_workspaces()
            .unwrap()
            .into_iter()
            .map(|workspace| workspace.id)
            .collect::<Vec<_>>();
        ProjectApi::new(
            pool.clone(),
            data_root.join("sessions"),
            crate::clock::SystemClock,
            EffectWorkerHandle::unwatched(),
        )
        .create(CreateProjectRequest {
            name: name.to_string(),
            main_workspace_path: workspace_root.to_string_lossy().into_owned(),
        })
        .unwrap();
        SqliteWorkspaceRepository::new(pool.clone())
            .list_all_workspaces()
            .unwrap()
            .into_iter()
            .map(|workspace| workspace.id)
            .find(|id| !existing.contains(id))
            .expect("project creation adds one Workspace")
    }

    /// Publishes one Local Skill source, which also selects it into every Workspace's Desired set.
    fn select_grilling(
        repository: &SqliteEffectRepository,
        workspace_id: &WorkspaceId,
        catalog: &Path,
        published_at: i64,
    ) {
        fs::create_dir_all(catalog).unwrap();
        fs::write(catalog.join("SKILL.md"), MANIFEST).unwrap();
        let name = SkillName::parse("grilling").unwrap();
        let key = SkillSelectionKey::new(SourceKind::Local, Namespace::local(), name.clone());
        let state = DesiredSkillState::try_new(SkillState {
            name,
            skill_md_digest: Digest::sha256(MANIFEST.as_bytes()),
            source: SkillSource::Local {
                namespace: Namespace::local(),
                version: SourceVersion::parse("1").unwrap(),
            },
        })
        .unwrap();
        repository
            .publish_source(&state, catalog, SourcePublication::Create, published_at)
            .unwrap();
        // Asserting the coupling rather than replacing the spec: an install that stopped reaching
        // Desired would otherwise be masked by the test writing it by hand.
        assert_eq!(
            repository.load_workspace_effect(workspace_id).unwrap().spec,
            WorkspaceEffectSpec {
                skills: BTreeMap::from([(key, state)]),
            }
        );
    }

    /// The surface declarations one running Agent plugin publishes when it starts.
    fn agent_declarations() -> Vec<FilesystemSkillSurface> {
        vec![FilesystemSkillSurface {
            workspace_relative_path: SurfacePath::parse(".opencode/skills").unwrap(),
            materialization_format: MaterializationFormat::skill_directory_v1(),
            consumer: ConsumerId::new("official/ora-space.opencode"),
            coordination: ConsumerCoordination::WaitForIdleAndRestart,
        }]
    }

    /// Declares one Agent-consumed surface rooted at the given Workspace directory.
    fn declare_surface(
        repository: &SqliteEffectRepository,
        workspace_id: &WorkspaceId,
        workspace_root: &Path,
    ) {
        let descriptors = SurfaceDescriptorSet::merge(workspace_id, agent_declarations()).unwrap();
        repository
            .replace_surfaces(
                workspace_id,
                workspace_root,
                &descriptors,
                PUBLISHED_AT + 10,
            )
            .unwrap();
    }

    /// Claims the single request under test with a lease long enough to outlive the assertions.
    fn claim(repository: &SqliteEffectRepository, now: i64) -> ClaimedReconcile {
        repository
            .claim_due_reconcile_requests(WORKER, now, now + 60_000, 8)
            .unwrap()
            .remove(0)
    }

    /// Counts what is currently claimable, which is what a later pass would actually pick up.
    fn claimable(repository: &SqliteEffectRepository, now: i64) -> usize {
        let claimed = repository
            .claim_due_reconcile_requests("probe", now, now + 60_000, 8)
            .unwrap();
        for entry in &claimed {
            // Release the probe's claim so the assertion does not change what it measured.
            repository
                .retry_reconcile_request(&entry.claim, "probe", now, now)
                .unwrap();
        }
        claimed.len()
    }

    /// A Workspace created after the declaration still materializes, with no plugin restart.
    ///
    /// This is the exact shape of the original defect. Surface registration only ever ran when a
    /// plugin process started, so a Workspace created while that plugin was already running was
    /// offered no surface at all: its Desired set was complete and correct from the first moment —
    /// the `workspaces` insert trigger seeds it — but there was nothing to project it onto, so it
    /// never entered the reconcile queue and no amount of waiting materialized anything. Only a
    /// restart, by forcing the plugin to re-declare against a Workspace list that now included it,
    /// appeared to fix it.
    #[test]
    fn a_workspace_created_after_the_declaration_still_materializes() {
        let temp = TempDir::new().unwrap();
        let first_root = temp.path().join("workspace");
        let (pool, first_id) = fixture(temp.path(), &first_root);
        let repository = SqliteEffectRepository::new(pool.clone());
        let coordinator = RecordingCoordinator::default();
        select_grilling(
            &repository,
            &first_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &first_id, &first_root);
        // Drain the Workspace that existed when the plugin declared, so what remains claimable is
        // attributable only to the Workspace added afterwards.
        let first = claim(&repository, PUBLISHED_AT + 20);
        reconcile_one(&repository, &coordinator, first.due, PUBLISHED_AT + 20);
        repository
            .complete_reconcile_request(&first.claim, Generation::new(1), PUBLISHED_AT + 20)
            .unwrap();

        // The plugin keeps running and never declares again; a second Workspace appears now.
        let second_root = temp.path().join("workspace-2");
        let second_id = create_project_workspace(&pool, temp.path(), &second_root, "Second");
        assert_eq!(
            repository.list_workspaces_with_active_surfaces().unwrap(),
            BTreeSet::from([first_id]),
            "the new Workspace starts with no surface, which is what the defect never repaired",
        );
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 30),
            0,
            "with no surface the new Workspace owes no work at all, so nothing is merely pending",
        );

        let workspaces = SqliteWorkspaceRepository::new(pool)
            .list_all_workspaces()
            .unwrap();
        let converged = converge_workspace_surfaces(
            &repository,
            &workspaces,
            &agent_declarations(),
            PUBLISHED_AT + 40,
        )
        .unwrap();

        assert_eq!(converged, 1);
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 50),
            1,
            "convergence must leave the new Workspace owing exactly the reconcile it never had",
        );
        let second = claim(&repository, PUBLISHED_AT + 50);
        assert_eq!(second.due.workspace_id, second_id);
        let outcome = reconcile_one(&repository, &coordinator, second.due, PUBLISHED_AT + 50);
        assert_eq!(
            outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1),
            }
        );
        let materialized = second_root
            .join(".opencode")
            .join("skills")
            .join("grilling");
        assert_eq!(
            fs::read_to_string(materialized.join("SKILL.md")).unwrap(),
            MANIFEST
        );
        assert!(materialized.join(MARKER_FILE_NAME).exists());
    }

    /// Creating a Workspace wakes the worker, so convergence does not wait out a scan interval.
    ///
    /// Correctness never depends on this wake — the pass converges the same Workspace regardless —
    /// but creating a Workspace while a plugin is already running is the ordinary case, not an edge
    /// one, and leaving it to the next scan means the first prompt in a new task can run before its
    /// Skills exist.
    #[test]
    fn creating_a_project_wakes_the_effect_worker() {
        let temp = TempDir::new().unwrap();
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp.path().join("ora.sqlite3")),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let reconcile = EffectWorkerHandle::unwatched();
        assert!(!reconcile.is_pending());

        ProjectApi::new(
            pool,
            temp.path().join("sessions"),
            crate::clock::SystemClock,
            reconcile.clone(),
        )
        .create(CreateProjectRequest {
            name: "Demo".to_string(),
            main_workspace_path: workspace_root.to_string_lossy().into_owned(),
        })
        .unwrap();

        assert!(reconcile.is_pending());
    }

    /// One worker pass takes a late Workspace all the way from unregistered to files on disk.
    ///
    /// Two things are being pinned here. First, the worker itself performs the registration: the
    /// test above drives convergence directly, which proves the logic but would stay green if the
    /// worker stopped calling it, so this one goes through `run_pass`, the entry point production
    /// uses. Second, registration and materialization happen in the *same* pass — convergence runs
    /// before claiming and stamps `not_before_at` with that pass's own timestamp — which is what
    /// bounds the user-visible delay at one scan interval rather than two.
    #[test]
    fn one_worker_pass_registers_and_materializes_a_late_workspace() {
        let temp = TempDir::new().unwrap();
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp.path().join("ora.sqlite3")),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        let first_id =
            create_project_workspace(&pool, temp.path(), &temp.path().join("workspace"), "Demo");
        let plugin_host = Arc::new(
            PluginApi::open(
                pool.clone(),
                temp.path().to_path_buf(),
                PathBuf::from("deno"),
                crate::clock::SystemClock,
                AppEventHub::new().publisher(),
                Arc::new(UserConfigApi::new(pool.clone())),
            )
            .unwrap(),
        );
        // The declaration reaches only the Workspaces that exist at this moment.
        plugin_host
            .replace_agent_effect_surfaces(
                PluginId::new("official", "ora-space.opencode").unwrap(),
                agent_declarations(),
            )
            .unwrap();
        let repository = SqliteEffectRepository::new(pool.clone());
        // Real timestamps throughout, because `run_pass` reads the real clock: a Skill dated in the
        // far future would make every later row fail its `updated_at >= created_at` check.
        let installed_at = crate::clock::SystemClock.now_timestamp_millis();
        select_grilling(
            &repository,
            &first_id,
            &temp.path().join("catalog"),
            installed_at,
        );

        // The Skill is already installed and the plugin is already running when the Workspace
        // appears, which is exactly the ordering that used to materialize nothing until a restart.
        let second_root = temp.path().join("workspace-2");
        let second_id = create_project_workspace(&pool, temp.path(), &second_root, "Second");
        assert_eq!(
            repository.list_workspaces_with_active_surfaces().unwrap(),
            BTreeSet::from([first_id.clone()]),
        );

        // A current-thread runtime whose handle is used from outside it, exactly as `spawn` does:
        // the coordinator blocks on plugin IPC, which a runtime thread could not do.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        EffectWorker::new(pool, plugin_host, Arc::new(RecordingSessions::default()))
            .run_pass(runtime.handle());

        assert_eq!(
            repository.list_workspaces_with_active_surfaces().unwrap(),
            BTreeSet::from([first_id, second_id]),
        );
        let materialized = second_root
            .join(".opencode")
            .join("skills")
            .join("grilling");
        assert_eq!(
            fs::read_to_string(materialized.join("SKILL.md")).unwrap(),
            MANIFEST,
            "one pass must register the surface and materialize into it, not just the first half",
        );
        assert!(materialized.join(MARKER_FILE_NAME).exists());
    }

    /// The whole chain: a selected Skill reaches the declared surface and is marked Ora-owned.
    #[test]
    fn a_selected_skill_is_materialized_into_the_declared_surface() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);
        let coordinator = RecordingCoordinator::default();
        let request = claim(&repository, PUBLISHED_AT + 20);
        let surface_key = request.due.descriptor.surface_key.clone();

        let outcome = reconcile_one(&repository, &coordinator, request.due, PUBLISHED_AT + 20);
        assert_eq!(
            outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1),
            }
        );
        assert!(
            repository
                .complete_reconcile_request(&request.claim, Generation::new(1), PUBLISHED_AT + 20)
                .unwrap(),
        );

        let materialized = workspace_root
            .join(".opencode")
            .join("skills")
            .join("grilling");
        assert_eq!(
            fs::read_to_string(materialized.join("SKILL.md")).unwrap(),
            MANIFEST
        );
        // The ownership marker separates an Ora-managed target from Preserved State; without it a
        // materialized directory is indistinguishable from a Skill the user wrote themselves.
        assert!(materialized.join(MARKER_FILE_NAME).exists());
        assert_eq!(
            repository
                .load_managed_skills(&workspace_id, &surface_key)
                .unwrap()
                .len(),
            1,
            "materializing a target must record the ownership it just took",
        );
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 30),
            0,
            "a surface that reached its Desired generation owes no further reconcile",
        );
        assert_eq!(
            coordinator
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            vec![
                "quiesce:official/ora-space.opencode".to_string(),
                "resume:official/ora-space.opencode@1".to_string(),
            ],
            "the consumer is paused before the write and restarted onto the applied generation",
        );
    }

    /// A busy consumer defers the mutation instead of writing underneath a running turn.
    #[test]
    fn a_busy_consumer_defers_materialization_and_keeps_the_request() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);
        let coordinator = RecordingCoordinator {
            busy: true,
            calls: Mutex::default(),
        };

        let request = claim(&repository, PUBLISHED_AT + 20);
        let outcome = reconcile_one(&repository, &coordinator, request.due, PUBLISHED_AT + 20);

        // Waiting on a turn is an unmet precondition, not a failure: retrying sooner cannot help,
        // so the surface parks until the runtime or the safety scan says something changed.
        assert_eq!(
            outcome,
            SurfaceOutcome::Blocked {
                reason: "awaiting_external_change",
            }
        );
        // Scanning creates the surface root itself, so absence of the Skill — not of the
        // directory — is what proves the deferral held.
        assert!(
            !workspace_root
                .join(".opencode")
                .join("skills")
                .join("grilling")
                .exists()
        );
        assert!(
            repository
                .block_reconcile_request(
                    &request.claim,
                    "awaiting_external_change",
                    PUBLISHED_AT + 20
                )
                .unwrap(),
        );
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 30),
            0,
            "a blocked surface is not retried on a timer",
        );
        assert_eq!(
            repository
                .rearm_blocked_reconcile_requests(PUBLISHED_AT + 40)
                .unwrap(),
            1,
            "the safety scan is what recovers a runtime event lost before it arrived",
        );
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 50),
            1,
            "a re-armed surface becomes claimable again",
        );
        assert_eq!(
            coordinator
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            vec!["quiesce:official/ora-space.opencode".to_string()],
            "a consumer that never paused must never be resumed",
        );
    }

    /// Two workers must never hold the same surface, or two plans would hit the same targets.
    #[test]
    fn a_claimed_surface_is_invisible_to_a_second_worker_until_its_lease_expires() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);

        let first = repository
            .claim_due_reconcile_requests(WORKER, PUBLISHED_AT + 20, PUBLISHED_AT + 80, 8)
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(
            repository
                .claim_due_reconcile_requests("worker-2", PUBLISHED_AT + 30, PUBLISHED_AT + 90, 8)
                .unwrap()
                .is_empty(),
            "a live lease keeps a sibling worker off the surface",
        );

        // Past the lease, the surface must become claimable again: a worker that crashed mid-run
        // leaves its row claimed forever otherwise.
        let stolen = repository
            .claim_due_reconcile_requests("worker-2", PUBLISHED_AT + 100, PUBLISHED_AT + 160, 8)
            .unwrap();
        assert_eq!(stolen.len(), 1);
        assert_ne!(
            stolen[0].claim.token, first[0].claim.token,
            "taking over a surface must invalidate the previous owner's fence",
        );
    }

    /// A worker that lost its lease must not be able to write the outcome of stale work.
    #[test]
    fn a_stale_claim_can_no_longer_complete_block_or_reschedule() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);
        let stale = claim(&repository, PUBLISHED_AT + 20).claim;
        // A second worker takes over once the first lease has expired.
        let live = repository
            .claim_due_reconcile_requests(
                "worker-2",
                PUBLISHED_AT + 100_000,
                PUBLISHED_AT + 160_000,
                8,
            )
            .unwrap()
            .remove(0)
            .claim;

        assert!(
            !repository
                .renew_reconcile_claim(
                    &stale,
                    WORKER,
                    PUBLISHED_AT + 300_000,
                    PUBLISHED_AT + 100_010
                )
                .unwrap(),
            "renewal is the signal that tells a superseded worker to stop",
        );
        assert!(
            !repository
                .complete_reconcile_request(&stale, Generation::new(1), PUBLISHED_AT + 100_010)
                .unwrap(),
        );
        assert!(
            !repository
                .block_reconcile_request(&stale, "stale", PUBLISHED_AT + 100_010)
                .unwrap(),
        );
        assert!(
            !repository
                .retry_reconcile_request(
                    &stale,
                    "stale",
                    PUBLISHED_AT + 400_000,
                    PUBLISHED_AT + 100_010
                )
                .unwrap(),
        );
        assert!(
            repository
                .renew_reconcile_claim(
                    &live,
                    "worker-2",
                    PUBLISHED_AT + 400_000,
                    PUBLISHED_AT + 100_010
                )
                .unwrap(),
            "the current owner keeps its lease across the same window",
        );
    }

    /// A transient failure must wait out its persisted delay instead of spinning.
    #[test]
    fn a_scheduled_retry_is_not_claimable_before_its_delay_elapses() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);
        let request = claim(&repository, PUBLISHED_AT + 20);

        repository
            .retry_reconcile_request(
                &request.claim,
                "transient_failure",
                PUBLISHED_AT + 5_000,
                PUBLISHED_AT + 20,
            )
            .unwrap();

        assert_eq!(claimable(&repository, PUBLISHED_AT + 1_000), 0);
        assert_eq!(claimable(&repository, PUBLISHED_AT + 6_000), 1);
    }

    /// A newer Desired must clear an old backoff, because it may be exactly what fixes the failure.
    #[test]
    fn a_new_generation_re_arms_a_surface_that_was_waiting_out_a_backoff() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);
        let request = claim(&repository, PUBLISHED_AT + 20);
        repository
            .retry_reconcile_request(
                &request.claim,
                "transient_failure",
                PUBLISHED_AT + 1_000_000,
                PUBLISHED_AT + 20,
            )
            .unwrap();
        assert_eq!(claimable(&repository, PUBLISHED_AT + 30), 0);

        // Re-declaring the surface stands in for any committed change that raises the generation.
        declare_surface(&repository, &workspace_id, &workspace_root);

        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 40),
            1,
            "a committed change must not have to wait out the previous failure's delay",
        );
    }

    /// Startup must rebuild work whose only remaining evidence is unconverged durable state.
    #[test]
    fn recovery_rebuilds_a_request_for_a_surface_left_short_of_its_generation() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_grilling(
            &repository,
            &workspace_id,
            &temp.path().join("catalog"),
            PUBLISHED_AT,
        );
        declare_surface(&repository, &workspace_id, &workspace_root);
        // Losing the request is what a crash between commit and scheduling looks like.
        let request = claim(&repository, PUBLISHED_AT + 20);
        repository
            .complete_reconcile_request(&request.claim, Generation::new(1), PUBLISHED_AT + 20)
            .unwrap();
        assert_eq!(claimable(&repository, PUBLISHED_AT + 30), 0);

        assert_eq!(
            repository
                .recover_reconcile_requests(PUBLISHED_AT + 40)
                .unwrap(),
            1,
            "status still proves the surface never applied its generation",
        );
        assert_eq!(claimable(&repository, PUBLISHED_AT + 50), 1);
    }
}
