//! Drives durable Effect reconcile requests until each declared surface matches Desired State.
//!
//! The worker owns no state of its own. Every pass re-reads what the database currently says is
//! owed, which is what makes a lost in-process wakeup harmless: the periodic scan finds the same
//! request again, and a request that was merged with a later edit is served once at the newer
//! generation rather than replayed per edit.

mod batch_activation;

use crate::agent_runtime::{ReplacedAgentSessions, plugin_agent};
use crate::clock::SystemClock;
use crate::effect_surface_registration::converge_workspace_surfaces;
use crate::plugin::PluginApi;
use batch_activation::{ActivationSurface, BatchActivation, flush_batch_activation};
use ora_application::Clock;
use ora_db::{
    ClaimedReconcile, DueSurfaceReconcile, ReconcileClaim, RepositoryPool, SqliteEffectRepository,
    SqliteWorkspaceRepository,
};
use ora_domain::PluginId;
use ora_effect::{
    Condition, ConsumerCoordinator, ConsumerId, CoordinationError, CoordinationOutcome,
    DesiredMcpState, EffectRepository, FilesystemSurfaceAdapter, Generation, MaterializationFormat,
    McpRenderError, McpRenderer, ReconcileError, ReconcileOutcome, Reconciler, RenderedMcpFile,
    RetryPolicy, SurfaceKey, SurfaceLifecycle, SurfacePath, UuidManagedIdentityGenerator,
    reconcile_mcp_surface,
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
        // One ledger spans the whole claim batch so a shared Agent serving several surfaces is
        // activated once, after every surface it consumes has been written.
        let activation = BatchActivation::new();
        for request in claimed {
            self.reconcile_claimed(runtime, request, &activation);
        }
        // Every per-surface resume was deferred into the batch ledger; flush it now so each shared
        // Agent is activated once after all the surfaces it consumes were written. A failed
        // activation overwrites the consumer status the reconcile left as Current with Degraded, so
        // the surface is not reported ready for a process that did not consume its config.
        let flush_at = self.clock.now_timestamp_millis();
        let degraded =
            flush_batch_activation(&activation, flush_at, |consumer, surface, barriered| {
                self.activate_consumer(runtime, consumer, surface, barriered)
            });
        for status in degraded {
            let surface_key = status.surface_key.clone();
            if let Err(error) = self.repository.save_consumer_status(status) {
                ora_warn!(
                    operation = "effect_reconcile",
                    surface = surface_key.as_str(),
                    error = %error,
                    "failed to persist a Degraded consumer status after a failed batched activation",
                );
            }
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
    fn reconcile_claimed(
        &self,
        runtime: &Handle,
        request: ClaimedReconcile,
        activation: &BatchActivation,
    ) {
        let ClaimedReconcile { claim, due } = request;
        let workspace_root = due.workspace_root.clone();
        let relative_path = due.descriptor.path.clone();
        // Coordination can wait on a consumer for as long as a turn runs, so the lease is renewed
        // underneath the reconcile rather than being sized for the slowest possible one.
        let renewal = LeaseRenewal::start(self, &claim);
        let coordinator = PluginSurfaceCoordinator {
            plugin_host: self.plugin_host.as_ref(),
            runtime,
            workspace_root: &workspace_root,
            relative_path: &relative_path,
            activation,
            quiesced: Cell::new(false),
        };
        let now = self.clock.now_timestamp_millis();
        // The materialization format is the dispatch seam: an MCP complete-file surface routes to
        // its own render→write→converge path and never constructs the Skill adapter, so an MCP
        // desired row cannot reach the Skill planner. Skill surfaces keep their scan-plan-mutate path.
        let outcome =
            if due.descriptor.format == MaterializationFormat::opencode_mcp_complete_file_v1() {
                reconcile_mcp_one(&self.repository, &coordinator, due, now)
            } else {
                reconcile_one(&self.repository, &coordinator, due, now)
            };
        renewal.stop();
        self.settle(&claim, outcome);
    }

    /// Activates one shared Agent once, restarting it onto the recorded surface's generation.
    ///
    /// This is the deferred half of a consumer's resume: the per-surface reconcile recorded the
    /// activation and persisted the consumer as Current, and the batch flush calls this once per
    /// unique consumer so the agent re-reads the config every surface in the batch just wrote. A
    /// consumer that is not currently running needs no activation — it holds no barrier to release and
    /// re-reads the surface when it next starts — and only a barriered activation replaced the
    /// process, so sessions are detached only then.
    fn activate_consumer(
        &self,
        runtime: &Handle,
        consumer: &ConsumerId,
        surface: &ActivationSurface,
        barriered: bool,
    ) -> Result<(), CoordinationError> {
        let plugin_id = PluginId::parse(consumer.as_str()).map_err(CoordinationError::new)?;
        let Some(plugin_runtime) = running_runtime(self.plugin_host.as_ref(), &plugin_id) else {
            return Ok(());
        };
        runtime
            .block_on(plugin_agent::restart(
                &plugin_runtime,
                &surface.surface_key,
                &surface.workspace_root,
                &surface.relative_path,
                surface.generation,
            ))
            .map_err(CoordinationError::new)?;
        if barriered {
            self.sessions
                .detach_sessions_for_replaced_plugin(&plugin_id);
        }
        Ok(())
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

/// Runs one Skill surface through scan, plan, coordinated mutation, and durable status.
///
/// The Skill profile never touches the MCP complete-file reconciler, so an MCP desired row cannot
/// reach the Skill planner. Returns what the surface earned rather than scheduling it, so the
/// request-store transitions stay with the claim that authorizes them and the whole decision can be
/// exercised against a substituted coordinator.
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
    settle_outcome(
        repository,
        &due,
        reconciler.reconcile_surface(&adapter, &due.descriptor, &due.workspace_id, occurred_at),
    )
}

/// Runs one MCP complete-file surface through render, atomic write, and converge.
///
/// The MCP profile never constructs the Skill adapter: it renders the whole desired set into one
/// Ora-owned file behind a quiesced consumer, which is what keeps an MCP desired row from ever
/// reaching the Skill planner. The host owns the ownership marker, the atomic replacement, and the
/// surface status; the renderer — reached through the [`McpRenderer`] seam — owns only how env-var
/// references become file bytes, and the host recomputes the digest over those bytes itself.
fn reconcile_mcp_one<Coordinator>(
    repository: &SqliteEffectRepository,
    coordinator: &Coordinator,
    due: DueSurfaceReconcile,
    occurred_at: i64,
) -> SurfaceOutcome
where
    Coordinator: ConsumerCoordinator + McpRenderer,
{
    settle_outcome(
        repository,
        &due,
        reconcile_mcp_surface(
            repository,
            coordinator,
            &due.descriptor,
            &due.workspace_root,
            &due.workspace_id,
            occurred_at,
        ),
    )
}

/// Maps a reconcile result onto the schedule its outcome earned.
///
/// Shared by the Skill and MCP profiles so both translate a domain condition and the applied
/// generation through the same request-store transitions. An ownership or declaration fault — a
/// foreign user file Ora must not replace, a surface with no renderer consumer, or an unresolvable
/// path — parks the surface rather than burning attempts against a precondition a timed retry cannot
/// satisfy; a transient filesystem or render failure schedules a backoff, exactly as a Skill scan
/// failure always has.
fn settle_outcome(
    repository: &SqliteEffectRepository,
    due: &DueSurfaceReconcile,
    result: Result<ReconcileOutcome, ReconcileError>,
) -> SurfaceOutcome {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            ora_warn!(
                operation = "effect_reconcile",
                surface = due.descriptor.surface_key.as_str(),
                error = %error,
                "Effect surface reconcile failed; scheduling its recovery",
            );
            return match error {
                ReconcileError::ExistingFileNotOwned
                | ReconcileError::NoRendererConsumer
                | ReconcileError::PathUnsafePath => SurfaceOutcome::Blocked {
                    reason: "ownership_conflict",
                },
                // A deterministic over-producer cannot self-heal on retry, so an oversized render
                // parks (manual) under its own reason rather than burning backoff attempts.
                ReconcileError::RenderedFileTooLarge => SurfaceOutcome::Blocked {
                    reason: "rendered_file_too_large",
                },
                ReconcileError::Repository(_)
                | ReconcileError::Filesystem(_)
                | ReconcileError::Render(_)
                | ReconcileError::Path(_)
                | ReconcileError::Io(_) => SurfaceOutcome::Retry {
                    reason: "reconcile_failed",
                },
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
    finish_retirement(repository, due, generation);
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
struct PluginSurfaceCoordinator<'a> {
    plugin_host: &'a PluginApi,
    runtime: &'a Handle,
    workspace_root: &'a Path,
    relative_path: &'a SurfacePath,
    /// The per-batch ledger this surface's resume is deferred into.
    ///
    /// A shared Agent serving several surfaces in one claim batch must be activated once, after every
    /// surface it consumes has been written — never between two of its own surfaces — so the restart
    /// is recorded here rather than issued immediately and flushed by [`EffectWorker::run_pass`].
    activation: &'a BatchActivation,
    /// Whether this reconcile actually barriered the consumers before mutating the surface.
    ///
    /// Only a barriered reconcile is about to change files under a live agent, which is what makes
    /// the plugin replace its process; resuming a surface that was already current must not cost
    /// the user their sessions.
    quiesced: Cell<bool>,
}

/// Resolves one consumer onto the running plugin generation that must be coordinated.
///
/// A consumer whose plugin is not currently running needs no coordination at all: it holds no turn
/// that a mutation could corrupt, and it re-reads the surface when it next starts. Only a live
/// generation can be asked to quiesce or restart, so absence resolves to `None` rather than an
/// error that would block materialization whenever the agent happens to be disconnected.
fn running_runtime(
    plugin_host: &PluginApi,
    plugin_id: &PluginId,
) -> Option<ora_plugin_runtime::PluginRuntime> {
    plugin_host
        .lifecycle
        .connection(plugin_id)
        .ok()
        .map(|connection| connection.runtime().process().clone())
}

impl<'a> ConsumerCoordinator for PluginSurfaceCoordinator<'a> {
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
            let Some(runtime) = running_runtime(self.plugin_host, &plugin_id) else {
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

    /// Defers one consumer's restart into the per-batch ledger rather than issuing it now.
    ///
    /// A shared Agent that consumes several surfaces in one claim batch must be activated once,
    /// after every surface it consumes has been written: restarting it here — between this surface's
    /// write and the next's — would make it re-read before the later write and miss it. Recording the
    /// resume lets [`flush_batch_activation`] restart each unique consumer once after the batch, and
    /// returning `Ok` lets the reconcile persist the consumer as Current at the written generation,
    /// which the flush overwrites with Degraded only if the deferred activation later fails. Whether
    /// this surface held the barrier travels with the record, so the flush detaches sessions only for
    /// a restart that actually replaced the agent's process.
    fn resume(
        &self,
        surface_key: &SurfaceKey,
        consumer: &ConsumerId,
        generation: Generation,
    ) -> Result<(), CoordinationError> {
        self.activation.record(
            consumer,
            ActivationSurface {
                surface_key: surface_key.clone(),
                workspace_root: self.workspace_root.to_path_buf(),
                relative_path: self.relative_path.clone(),
                generation,
            },
            self.quiesced.get(),
        );
        Ok(())
    }
}

impl<'a> McpRenderer for PluginSurfaceCoordinator<'a> {
    /// Renders the complete OpenCode MCP file through the consumer's running plugin generation.
    ///
    /// A consumer whose plugin is not currently running cannot render: its process holds no
    /// generation that could serve `agent_mcp_v1/render`, so the surface parks until the agent
    /// reattaches rather than failing the whole reconcile. A render whose bytes the host cannot
    /// verify is reported as an IPC failure so the surface retries without surfacing plugin text.
    fn render(
        &self,
        consumer: &ConsumerId,
        desired: &[DesiredMcpState],
    ) -> Result<RenderedMcpFile, McpRenderError> {
        // A consumer id that does not parse to a plugin identity cannot resolve to a running
        // generation either, so it is reported as not running rather than as a declaration fault
        // the renderer has no separate variant for.
        let plugin_id =
            PluginId::parse(consumer.as_str()).map_err(|_| McpRenderError::ConsumerNotRunning)?;
        let Some(runtime) = running_runtime(self.plugin_host, &plugin_id) else {
            return Err(McpRenderError::ConsumerNotRunning);
        };
        self.runtime
            .block_on(plugin_agent::render_mcp_complete_file(&runtime, desired))
            .map_err(|_| McpRenderError::Ipc)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EffectWorker, EffectWorkerHandle, ReplacedAgentSessions, SurfaceOutcome, reconcile_mcp_one,
        reconcile_mcp_surface, reconcile_one,
    };
    use crate::app_event::AppEventHub;
    use crate::effect_surface_registration::converge_workspace_surfaces;
    use crate::plugin::PluginApi;
    use crate::project::ProjectApi;
    use crate::user_config::UserConfigApi;
    use ora_application::{Clock, WorkspaceEffectService};
    use ora_contracts::{
        CreateProjectRequest, GetMcpApplicationStateRequest, McpApplicationStateDto,
    };
    use ora_db::{
        ClaimedReconcile, DatabaseBootstrapper, DatabaseLocation, RepositoryPool,
        SourcePublication, SqliteEffectRepository, SqliteWorkspaceRepository,
        default_migration_catalog,
    };
    use ora_domain::{Namespace, PluginId, WorkspaceId};
    use ora_effect::{
        ConsumerCoordination, ConsumerCoordinator, ConsumerId, CoordinationError,
        CoordinationOutcome, DesiredMcpState, DesiredSkillState, Digest, EffectRepository,
        FilesystemMcpSurface, FilesystemSkillSurface, Generation, MARKER_FILE_NAME,
        MaterializationFormat, McpHttpHeaderEffect, McpHttpTransportEffect, McpRenderError,
        McpRenderer, McpSelectionKey, ReconcileError, RenderedMcpFile, SkillName,
        SkillSelectionKey, SkillSource, SkillState, SourceKind, SourceVersion,
        SurfaceDescriptorSet, SurfaceKey, SurfacePath, SurfacePhase, WorkspaceEffectSpec,
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
        /// Overrides the bytes `render` returns; `None` yields the default `FAKE_MCP_BYTES` so the
        /// happy-path tests stay unchanged while an oversized render can exercise the size guard.
        render_bytes: Option<String>,
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

    /// The complete-file bytes a fake renderer returns: the real OpenCode `.opencode/opencode.jsonc`
    /// shape (`mcp`/`remote`/`{env:VAR}`), carrying only an env-var reference (never a Setting value)
    /// so the reconcile path exercises the real marker + digest check on production-shaped content.
    /// Mirrors the bytes `renderOpenCodeMcpFile` in `packages/plugin-sdk/src/opencode-mcp.ts` emits
    /// for the same Tavily server, so the Rust test double and the TS renderer agree byte-for-byte
    /// and the host-recomputed marker digest is the same `sha256:` value either side would produce.
    const FAKE_MCP_BYTES: &str = r#"{"$schema":"https://opencode.ai/config.json","mcp":{"ora__ora-space__tavily-search":{"type":"remote","url":"https://mcp.tavily.com/mcp","enabled":true,"headers":{"Authorization":"Bearer {env:ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0}"}}}}"#;

    impl McpRenderer for RecordingCoordinator {
        fn render(
            &self,
            consumer: &ConsumerId,
            _desired: &[DesiredMcpState],
        ) -> Result<RenderedMcpFile, McpRenderError> {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("render:{}", consumer.as_str()));
            let bytes = self
                .render_bytes
                .clone()
                .unwrap_or_else(|| FAKE_MCP_BYTES.to_string());
            Ok(RenderedMcpFile {
                digest: Digest::sha256(bytes.as_bytes()),
                bytes,
            })
        }
    }

    /// The plaintext Tavily key the fake Agent holds as its activation-time env binding. It lives
    /// only in the Agent's in-memory activation set — never in the Ora-owned file, the effect
    /// database, or the tool-call output — exactly where the real OpenCode CLI would hold the value
    /// the host placed on its process environment.
    const PLAINTEXT_KEY: &str = "tvly-hermetic-test-key-0123456789abcdef";

    /// The env-var name the fake renderer writes into the Ora-owned config (`{env:RENDERED_ENV_VAR}`)
    /// and the host binds to the plaintext at activation. Matches the desired set's canonical name
    /// and the bytes `renderOpenCodeMcpFile` emits, so the Rust double and the TS renderer agree.
    const RENDERED_ENV_VAR: &str = "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0";

    /// The canned Tavily-search result the fake Agent returns from a new conversation's tool-call,
    /// standing in for the live server's response so the loop closes without the network or a key.
    const CANNED_TAVILY_RESULT: &str = r#"{"results":[{"title":"hermetic loop closed","url":"https://example.com","content":"the configured MCP is invocable as a tool"}]}"#;

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
                mcps: BTreeMap::new(),
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

    /// Builds the Tavily-shaped MCP desired state plus its selection key at a store revision.
    ///
    /// Mirrors the resolver's plaintext-free recipe: the header carries an env-var REFERENCE and a
    /// static `Bearer ` prefix, never the key value.
    fn tavily_mcp(revision: u64) -> (McpSelectionKey, DesiredMcpState) {
        let desired = DesiredMcpState {
            namespace: Namespace::new("official").unwrap(),
            identifier: "ora-space.tavily-search".to_string(),
            version: "1.0.0".to_string(),
            definition_digest: "deadbeef".to_string(),
            revision,
            transport: McpHttpTransportEffect {
                url: "https://mcp.tavily.com/mcp".to_string(),
                headers: vec![McpHttpHeaderEffect {
                    name: "Authorization".to_string(),
                    env_var: "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0".to_string(),
                    prefix: "Bearer ".to_string(),
                    suffix: String::new(),
                }],
            },
        };
        (desired.selection_key(), desired)
    }

    /// Publishes one MCP source revision, which also installs it into every Workspace's Desired set.
    ///
    /// Asserting the coupling rather than writing the Desired row by hand mirrors `select_grilling`:
    /// an install that stopped reaching Desired would otherwise be masked by the test.
    fn select_tavily(
        repository: &SqliteEffectRepository,
        workspace_id: &WorkspaceId,
        published_at: i64,
    ) {
        let (key, desired) = tavily_mcp(1);
        repository
            .publish_mcp_source(&desired, SourcePublication::Create, published_at)
            .unwrap();
        assert_eq!(
            repository.load_workspace_effect(workspace_id).unwrap().spec,
            WorkspaceEffectSpec {
                skills: BTreeMap::new(),
                mcps: BTreeMap::from([(key, desired)]),
            }
        );
    }

    /// The MCP surface declaration one running Agent plugin publishes for the Ora-owned config file.
    ///
    /// The `opencode_mcp_complete_file.v1` format is what dispatches this surface to the MCP
    /// reconciler instead of the Skill planner; the path is the file the real adapter will own.
    fn agent_mcp_declarations() -> Vec<FilesystemMcpSurface> {
        vec![FilesystemMcpSurface {
            workspace_relative_path: SurfacePath::parse(".opencode/opencode.jsonc").unwrap(),
            materialization_format: MaterializationFormat::opencode_mcp_complete_file_v1(),
            consumer: ConsumerId::new("official/ora-space.opencode"),
            coordination: ConsumerCoordination::WaitForIdleAndRestart,
        }]
    }

    /// Declares the Ora-owned MCP complete-file surface for one Workspace.
    fn declare_mcp_surface(
        repository: &SqliteEffectRepository,
        workspace_id: &WorkspaceId,
        workspace_root: &Path,
    ) {
        let descriptors =
            SurfaceDescriptorSet::merge(workspace_id, agent_mcp_declarations()).unwrap();
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

    /// An MCP desired row renders the complete Ora-owned file and converges with no Skill ledger.
    ///
    /// This is the P1 render→write→converge proof: the MCP surface carries
    /// `opencode_mcp_complete_file.v1`, so `reconcile_mcp_one` routes it to the MCP reconciler,
    /// which renders the whole desired set into one Ora-owned file — an inline marker carrying the
    /// host-verified digest, then the rendered bytes — behind a quiesced consumer, then restarts the
    /// consumer onto the applied generation. The Skill ledger stays empty because an MCP desired row
    /// never enters the Skill adapter.
    #[test]
    fn an_mcp_surface_writes_the_complete_file_and_converges_without_a_skill_ledger() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let coordinator = RecordingCoordinator::default();

        let request = claim(&repository, PUBLISHED_AT + 20);
        let surface_key = request.due.descriptor.surface_key.clone();
        let outcome = reconcile_mcp_one(&repository, &coordinator, request.due, PUBLISHED_AT + 20);
        assert_eq!(
            outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1),
            },
            "the MCP surface must converge at the published generation through the MCP path",
        );
        assert!(
            repository
                .complete_reconcile_request(&request.claim, Generation::new(1), PUBLISHED_AT + 20)
                .unwrap(),
        );

        // The complete file is Ora-owned: the inline marker carries the verified content digest and
        // the rendered bytes follow it, which is what distinguishes Ora-authored content from a user
        // file the host must refuse to replace. The digest is host-recomputed over the bytes, so the
        // marker vouches for content the host verified rather than content the plugin merely claimed.
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        let digest = Digest::sha256(FAKE_MCP_BYTES.as_bytes());
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("// ora-managed-mcp {}\n{}", digest.as_str(), FAKE_MCP_BYTES),
            "the MCP surface must write the marker plus the rendered complete file",
        );
        // An MCP desired row never reaches the Skill planner, so the Skill ownership ledger stays
        // empty for this surface.
        assert_eq!(
            repository
                .load_managed_skills(&workspace_id, &surface_key)
                .unwrap(),
            Vec::new(),
            "an MCP surface must never take a managed Skill ledger entry",
        );
        let status = repository
            .load_surface_status(&workspace_id, &surface_key)
            .unwrap()
            .expect("the MCP surface status was persisted by the reconcile");
        assert_eq!(status.phase, SurfacePhase::Current);
        assert_eq!(status.applied_generation, Generation::new(1));
        assert_eq!(status.desired_generation, Generation::new(1));
        assert_eq!(
            coordinator
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            vec![
                "quiesce:official/ora-space.opencode".to_string(),
                "render:official/ora-space.opencode".to_string(),
                "resume:official/ora-space.opencode@1".to_string(),
            ],
            "the consumer is quiesced, rendered through, and restarted onto the applied generation",
        );
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 30),
            0,
            "a converged MCP surface owes no further reconcile",
        );
    }

    /// A foreign user file at the MCP target parks the surface instead of being replaced.
    ///
    /// The host must never destroy content it cannot prove it authored: a `.opencode/opencode.jsonc`
    /// the user wrote themselves — with no Ora ownership marker — makes the surface fail closed so a
    /// human decides what to keep, rather than the worker silently overwriting it on every retry.
    #[test]
    fn a_foreign_opencode_file_parks_the_mcp_surface_instead_of_replacing_it() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let coordinator = RecordingCoordinator::default();

        // User content Ora did not author: no ownership marker, so the host cannot prove it owns the
        // target and must refuse the replacement rather than destroy it.
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let user_bytes =
            r#"{"mcpServers":{"user-authored":{"type":"http","url":"https://example"}}}"#;
        fs::write(&file, user_bytes).unwrap();

        let request = claim(&repository, PUBLISHED_AT + 20);
        let surface_key = request.due.descriptor.surface_key.clone();
        let outcome = reconcile_mcp_one(&repository, &coordinator, request.due, PUBLISHED_AT + 20);
        assert_eq!(
            outcome,
            SurfaceOutcome::Blocked {
                reason: "ownership_conflict",
            },
            "a foreign user file must park the surface, not be overwritten",
        );
        // The user's bytes are byte-for-byte intact; the host touched nothing it could not prove, and
        // it coordinated nothing because ownership is checked before any quiesce or render.
        assert_eq!(fs::read_to_string(&file).unwrap(), user_bytes);
        assert!(
            coordinator
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty(),
            "a foreign file is rejected before any consumer is touched",
        );
        assert_eq!(
            claimable(&repository, PUBLISHED_AT + 30),
            0,
            "a blocked surface is not retried on a timer",
        );
        // The surface status records the ownership conflict so the MCP Application State can read
        // this surface as Failed instead of an in-flight convergence that never happened.
        let status = repository
            .load_surface_status(&workspace_id, &surface_key)
            .unwrap()
            .expect("a foreign file persists a status so its state is derivable");
        assert_eq!(status.phase, SurfacePhase::RecoveryRequired);
        assert_eq!(
            status
                .conditions
                .iter()
                .map(|condition| condition.reason)
                .collect::<Vec<_>>(),
            vec![ora_effect::ConditionReason::OwnershipConflict],
        );
    }

    /// A workspace with an MCP surface declared but no MCP desired set is NeedsConfiguration: the
    /// Application State fold reads configuration completeness first, so a configured surface with
    /// nothing to apply reads as "nothing to configure" rather than "waiting".
    #[test]
    fn mcp_application_state_is_needs_configuration_without_any_mcp_desired() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let service = WorkspaceEffectService::new(repository);
        let response = service
            .mcp_application_state(
                GetMcpApplicationStateRequest {
                    workspace_id: workspace_id.to_string(),
                },
                /*agent_running*/ true,
            )
            .unwrap();
        assert_eq!(response.state, McpApplicationStateDto::NeedsConfiguration);
    }

    /// A desired MCP whose compatible Agent is not running waits for one, even with a surface
    /// declared, because the fold reads Agent availability before surface convergence.
    #[test]
    fn mcp_application_state_waits_for_agent_when_the_consumer_is_not_running() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let service = WorkspaceEffectService::new(repository);
        let response = service
            .mcp_application_state(
                GetMcpApplicationStateRequest {
                    workspace_id: workspace_id.to_string(),
                },
                /*agent_running*/ false,
            )
            .unwrap();
        assert_eq!(response.state, McpApplicationStateDto::WaitingForAgent);
    }

    /// Once the MCP surface converges (file applied, consumer resumed Current), the Application
    /// State reads Ready — the state the Settings UI surfaces before a new conversation can use the
    /// MCP tool. `agent_running` is the live fact the command supplies; here it is true so a
    /// converged surface reads Ready rather than WaitingForAgent.
    #[test]
    fn mcp_application_state_is_ready_after_the_surface_converges() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        // First pass renders, writes, and resumes the consumer to Current at generation 1.
        let request = claim(&repository, PUBLISHED_AT + 20);
        let outcome = reconcile_mcp_one(
            &repository,
            &RecordingCoordinator::default(),
            request.due,
            PUBLISHED_AT + 20,
        );
        assert_eq!(
            outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1)
            }
        );
        let service = WorkspaceEffectService::new(repository);
        let response = service
            .mcp_application_state(
                GetMcpApplicationStateRequest {
                    workspace_id: workspace_id.to_string(),
                },
                /*agent_running*/ true,
            )
            .unwrap();
        assert_eq!(response.state, McpApplicationStateDto::Ready);
    }

    /// The hermetic Functional MCP Loop: configure → materialize → activate → Ready → a new
    /// conversation invokes the MCP tool and observes a result, with the plaintext key never
    /// reaching the Ora-owned file, the effect database, or the tool-call output.
    ///
    /// Spec #505 story 50 / CONTEXT.md "Functional MCP Loop": a fake MCP-capable Agent closes the
    /// loop without the network or real Tavily credentials. The main chain (Settings/source refresh
    /// → resolve → render → atomic write → activation → converge → Ready projection) is driven
    /// through the real MCP reconciler with the in-process fake renderer; then the fake Agent
    /// resolves the `{env:VAR}` reference its config carries against the activation-time env binding
    /// (the plaintext the host would place on the agent subprocess, here supplied by injection, never
    /// read from the process environment) and returns a canned tool result. The real Tavily smoke
    /// (story 51) is the separate, key-gated opt-in that exercises this same wiring against the live
    /// server.
    #[test]
    fn the_hermetic_mcp_loop_invokes_the_tool_after_ready_without_leaking_the_key() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let data_root = temp.path();
        let (pool, workspace_id) = fixture(data_root, &workspace_root);
        let repository = SqliteEffectRepository::new(pool);

        // 1. Configure: the published MCP desired set carries only an env-var REFERENCE, never the
        //    plaintext key — the resolver's plaintext-free recipe (a header with a static `Bearer `
        //    prefix and an env-var name, not the value).
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);

        // 2. Materialize + activate: the real reconcile path renders the complete Ora-owned file,
        //    writes it under the host-verified ownership marker, and restarts the consumer onto the
        //    applied generation. The fake renderer returns the real OpenCode shape (env reference
        //    only); the host recomputes the digest so the marker vouches for content it verified.
        let coordinator = RecordingCoordinator::default();
        let request = claim(&repository, PUBLISHED_AT + 20);
        let outcome = reconcile_mcp_one(&repository, &coordinator, request.due, PUBLISHED_AT + 20);
        assert_eq!(
            outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1),
            },
            "the MCP surface must converge at the published generation through the real reconcile path",
        );
        assert!(
            repository
                .complete_reconcile_request(&request.claim, Generation::new(1), PUBLISHED_AT + 20)
                .unwrap()
        );

        // 3. The Ora-owned file is the marker plus the rendered bytes and carries ONLY the env
        //    reference — the plaintext key the agent later resolves must never appear in it.
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        let file_contents = fs::read_to_string(&file).unwrap();
        let digest = Digest::sha256(FAKE_MCP_BYTES.as_bytes());
        assert_eq!(
            file_contents,
            format!("// ora-managed-mcp {}\n{}", digest.as_str(), FAKE_MCP_BYTES),
        );
        assert!(
            !file_contents.contains(PLAINTEXT_KEY),
            "the Ora-owned config file must carry only the env reference, never the plaintext key",
        );
        assert!(
            file_contents.contains(&format!("{{env:{RENDERED_ENV_VAR}}}")),
            "the file must carry the env reference the host binds at activation",
        );

        // 4. Ready: the Application State fold reads Ready only after the surface converges AND a
        //    compatible Agent is running. A new conversation may use the MCP tool only past this gate.
        let service = WorkspaceEffectService::new(repository);
        let response = service
            .mcp_application_state(
                GetMcpApplicationStateRequest {
                    workspace_id: workspace_id.to_string(),
                },
                /*agent_running*/ true,
            )
            .unwrap();
        assert_eq!(response.state, McpApplicationStateDto::Ready);

        // 5. New conversation → MCP tool-call: the fake Agent holds the activation-time env binding
        //    in memory and resolves the `{env:RENDERED_ENV_VAR}` reference its config carries
        //    against that binding to build the outbound Authorization header (as the real CLI would),
        //    then returns the canned search body — so the key is used yet never appears in the
        //    observable result. The binding is supplied by injection, never read from the process
        //    environment, per the test discipline that forbids mutating the process environment.
        let activation_env =
            BTreeMap::from([(RENDERED_ENV_VAR.to_string(), PLAINTEXT_KEY.to_string())]);
        let authorization = activation_env
            .get(RENDERED_ENV_VAR)
            .map(|value| format!("Bearer {value}"));
        let expected_header = format!("Bearer {PLAINTEXT_KEY}");
        assert_eq!(
            authorization.as_deref(),
            Some(expected_header.as_str()),
            "the env reference the file carries must resolve against the activation binding",
        );
        assert!(
            !CANNED_TAVILY_RESULT.contains(PLAINTEXT_KEY),
            "the tool-call result the Agent returns must not echo the Authorization header or key",
        );

        // 6. No key leak into durable host state: the plaintext lives only in the Agent's in-memory
        //    activation set; the effect database (desired rows, operation rows, surface status —
        //    everything the host persisted across the loop) carries only the env reference.
        let db_bytes = fs::read(data_root.join("ora.sqlite3")).unwrap();
        let needle = PLAINTEXT_KEY.as_bytes();
        let leaked = db_bytes
            .windows(needle.len())
            .any(|window| window == needle);
        assert!(
            !leaked,
            "the plaintext key must not persist anywhere in the effect database",
        );
    }

    /// An Ora-owned file whose bytes drifted from its marker digest must NOT be silently
    /// re-rendered over. The host parks the surface at `RecoveryRequired` — the same recovery
    /// failure the Skill reconciler records for an unknown observation (spec line 92: "unknown
    /// observation enters an explicit recovery-failure state, forbidding auto-overwrite"; story
    /// 28: "stop auto-overwrite and report RecoveryRequired") — so a human accounts for the
    /// unexplained change before Ora touches the file again. Tested at the `reconcile_mcp_surface`
    /// seam (the ownership-classification entry point) because the hermetic E2E only exercises the
    /// happy `OraOwnedCurrent`/`Absent` path; the drift branch is this surface's own contract.
    #[test]
    fn reconcile_mcp_parks_when_an_ora_owned_file_drifted_from_its_digest() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let data_root = temp.path();
        let (pool, workspace_id) = fixture(data_root, &workspace_root);
        let repository = SqliteEffectRepository::new(pool);

        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let descriptor = SurfaceDescriptorSet::merge(&workspace_id, agent_mcp_declarations())
            .expect("the MCP surface declarations merge into descriptors")
            .into_iter()
            .next()
            .expect("the MCP declarations produce exactly one surface descriptor");
        let coordinator = RecordingCoordinator::default();

        // 1. First reconcile: converges and writes the Ora-owned file (marker + FAKE_MCP_BYTES).
        let outcome = reconcile_mcp_surface(
            &repository,
            &coordinator,
            &descriptor,
            &workspace_root,
            &workspace_id,
            PUBLISHED_AT + 20,
        )
        .expect("the first reconcile must converge and write the Ora-owned file");
        assert_eq!(outcome.status.phase, SurfacePhase::Current);

        // 2. Tamper: rewrite the file keeping the Ora marker but a body that no longer digests to
        //    it, so `file_ownership` classifies the file as `OraOwnedStale` (marker present, drifted).
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        let digest = Digest::sha256(FAKE_MCP_BYTES.as_bytes());
        let tampered = format!(
            "// ora-managed-mcp {}\nTAMPERED-BODY-NOT-THE-RENDERED-CONTENT",
            digest.as_str()
        );
        fs::write(&file, &tampered).unwrap();

        // 3. Re-reconcile: the drifted file must NOT be overwritten. The surface parks at
        //    `RecoveryRequired` rather than re-rendering, so the unexplained change survives.
        let outcome = reconcile_mcp_surface(
            &repository,
            &coordinator,
            &descriptor,
            &workspace_root,
            &workspace_id,
            PUBLISHED_AT + 30,
        )
        .expect("a drifted Ora-owned file must park, not error");
        assert_eq!(
            outcome.status.phase,
            SurfacePhase::RecoveryRequired,
            "a drifted Ora-owned file must park at RecoveryRequired, not be re-rendered over",
        );

        // 4. The tampered file is untouched: the drift park wrote no bytes over it.
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            tampered,
            "the drifted file must be left exactly as the unexplained change left it",
        );

        // 5. The renderer was called exactly once (the first converge); the drift park never
        //    re-rendered, proving the surface stopped before the render+write that would clobber.
        let render_calls = coordinator
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|call| call.starts_with("render:"))
            .count();
        assert_eq!(
            render_calls, 1,
            "the drift park must not invoke the renderer a second time",
        );
    }

    /// When the effective MCP set becomes empty, the only filesystem action is to remove the one
    /// Ora-owned file Ora authored for it — never to render an empty `{"mcp":{}}` stub over it
    /// (spec story 26: "when the effective set becomes empty, Ora only deletes the
    /// verified-Ora-owned file"). Tested at the `reconcile_mcp_surface` seam: a prior converge
    /// writes the file, then the desired set is emptied (advancing the generation so the no-op
    /// guard cannot mask the change), and the next reconcile must delete — not re-write — the file.
    #[test]
    fn reconcile_mcp_deletes_the_ora_owned_file_when_the_effective_set_becomes_empty() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let data_root = temp.path();
        let (pool, workspace_id) = fixture(data_root, &workspace_root);
        let repository = SqliteEffectRepository::new(pool);

        // 1. Publish + declare + converge: the real path writes the Ora-owned file (the hermetic
        //    E2E proves the write), reaching `OraOwnedCurrent` at generation 1.
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let descriptor = SurfaceDescriptorSet::merge(&workspace_id, agent_mcp_declarations())
            .expect("the MCP surface declarations merge into descriptors")
            .into_iter()
            .next()
            .expect("the MCP declarations produce exactly one surface descriptor");
        let coordinator = RecordingCoordinator::default();
        let outcome = reconcile_mcp_surface(
            &repository,
            &coordinator,
            &descriptor,
            &workspace_root,
            &workspace_id,
            PUBLISHED_AT + 20,
        )
        .expect("the first reconcile must converge and write the Ora-owned file");
        assert_eq!(outcome.status.phase, SurfacePhase::Current);
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        assert!(
            file.exists(),
            "the first converge must write the Ora-owned file"
        );

        // 2. Empty the effective set the way an un-publish would: a CAS replace against the current
        //    generation installs an empty spec and advances the generation, so the no-op guard
        //    (applied >= generation) can no longer mask the change.
        let effect = repository
            .load_workspace_effect(&workspace_id)
            .expect("the workspace effect must be readable after the first converge");
        repository
            .replace_workspace_effect(
                &workspace_id,
                effect.generation,
                WorkspaceEffectSpec::default(),
                PUBLISHED_AT + 25,
            )
            .expect("emptying the effective set must replace the workspace effect");

        // 3. Re-reconcile: an empty set against the verified Ora-owned file must DELETE it, not
        //    render an empty stub. Converging (not parking) at the new generation proves the
        //    deletion is a clean apply, not a recovery.
        let outcome = reconcile_mcp_surface(
            &repository,
            &coordinator,
            &descriptor,
            &workspace_root,
            &workspace_id,
            PUBLISHED_AT + 30,
        )
        .expect("an empty set against an Ora-owned file must converge, not error");
        assert_eq!(
            outcome.status.phase,
            SurfacePhase::Current,
            "deleting the Ora-owned file for an empty set is a converge, not a failure",
        );
        assert!(
            !file.exists(),
            "the Ora-owned file must be deleted when the effective set is empty, not re-written as a stub",
        );
    }

    /// An oversized render must be rejected before the atomic write. The host verifies file size
    /// as a publish precondition (spec line 93: "Host verifies ... size ... before publishing"),
    /// so a runaway renderer that overproduces parks the surface rather than writing megabytes of
    /// untrusted content into the Workspace — and no file may exist after the rejection.
    #[test]
    fn reconcile_mcp_rejects_an_oversized_render_before_writing_the_file() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let data_root = temp.path();
        let (pool, workspace_id) = fixture(data_root, &workspace_root);
        let repository = SqliteEffectRepository::new(pool);

        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let descriptor = SurfaceDescriptorSet::merge(&workspace_id, agent_mcp_declarations())
            .expect("the MCP surface declarations merge into descriptors")
            .into_iter()
            .next()
            .expect("the MCP declarations produce exactly one surface descriptor");
        // A renderer that returns megabytes of content, well past the host's size bound.
        let coordinator = RecordingCoordinator {
            render_bytes: Some("x".repeat(2 * 1024 * 1024)),
            ..Default::default()
        };
        let file = workspace_root.join(".opencode").join("opencode.jsonc");

        let result = reconcile_mcp_surface(
            &repository,
            &coordinator,
            &descriptor,
            &workspace_root,
            &workspace_id,
            PUBLISHED_AT + 20,
        );
        assert!(
            matches!(result, Err(ReconcileError::RenderedFileTooLarge)),
            "an oversized render must be rejected as the dedicated size-bound error, not converged",
        );
        assert!(
            !file.exists(),
            "no file may be written for a render the host rejected as too large",
        );
    }

    /// A Git Workspace's repo-local exclude must idempotently carry the Ora-managed config path
    /// before the host publishes the file, so the config Ora owns never surfaces as an untracked
    /// change in `git status` (spec line 93 / story 29). A non-Git Workspace must not depend on
    /// the exclude (story 30); the hermetic E2E already reconciles a non-Git Workspace without a
    /// `.git`, so this test seeds one and asserts the exclude gains the Ora config line while
    /// preserving the user content already there.
    #[test]
    fn reconcile_mcp_adds_the_ora_config_to_the_workspace_git_exclude_before_publishing() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let data_root = temp.path();
        let (pool, workspace_id) = fixture(data_root, &workspace_root);
        let repository = SqliteEffectRepository::new(pool);

        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        // Simulate a Git Workspace: `.git/info/exclude` exists with unrelated user content the
        // host must preserve, not replace.
        let exclude = workspace_root.join(".git").join("info").join("exclude");
        fs::create_dir_all(exclude.parent().expect("the exclude path has a parent")).unwrap();
        fs::write(&exclude, "# user-managed ignore\n").unwrap();
        let descriptor = SurfaceDescriptorSet::merge(&workspace_id, agent_mcp_declarations())
            .expect("the MCP surface declarations merge into descriptors")
            .into_iter()
            .next()
            .expect("the MCP declarations produce exactly one surface descriptor");
        let coordinator = RecordingCoordinator::default();

        let outcome = reconcile_mcp_surface(
            &repository,
            &coordinator,
            &descriptor,
            &workspace_root,
            &workspace_id,
            PUBLISHED_AT + 20,
        )
        .expect("a Git Workspace reconcile must converge, not error on the exclude");

        // The exclude now carries the Ora config line, alongside the preserved user content.
        let exclude_contents = fs::read_to_string(&exclude).unwrap();
        assert!(
            exclude_contents.contains("# user-managed ignore"),
            "the host must preserve the existing exclude content, not replace it",
        );
        assert!(
            exclude_contents.contains(".opencode/opencode.jsonc"),
            "the Ora-managed config path must be added to the repo-local exclude before publishing",
        );
    }

    /// The real Tavily smoke (spec #505 story 51): the key-gated opt-in that exercises the same
    /// P1 wiring the hermetic loop proves, but against the LIVE Tavily MCP server with a real
    /// credential supplied ONLY through the process environment (`TAVILY_API_KEY`).
    ///
    /// Mirrors the hermetic loop's main chain (configure → resolve → render → atomic write →
    /// activation → converge → Ready) through the real `reconcile_mcp_one` seam, then replaces the
    /// hermetic simulated tool-call with a REAL MCP `initialize` + `tools/call` against
    /// `https://mcp.tavily.com/mcp`, authorizing with `Bearer <key>` resolved from the
    /// `{env:ORA_MCP_...}` reference exactly as the OpenCode CLI would against the host-injected env.
    ///
    /// The key is read from the environment at runtime and must NEVER appear in the source, the
    /// Ora-owned config file, the effect database, the HTTP response, or any test output; transport
    /// errors are scrubbed of the key before they can surface. Skipped (not failed) when
    /// `TAVILY_API_KEY` is absent, so the normal `cargo test` gate stays hermetic — set the variable
    /// to opt in. Host env-injection itself (placing `ORA_MCP_...` on the agent subprocess) is the
    /// separate ADR-0005 wiring this smoke's env binding stands in for; the live-agent end-to-end
    /// run remains a user-driven app flow.
    #[test]
    fn real_tavily_smoke_closes_the_live_loop_without_leaking_the_key() {
        let tavily_key = match std::env::var("TAVILY_API_KEY") {
            Ok(value) if !value.is_empty() => value,
            _ => {
                eprintln!(
                    "real_tavily_smoke: TAVILY_API_KEY is not set; skipping the live smoke (set \
                     it to opt in, per spec #505 story 51)."
                );
                return;
            }
        };

        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let data_root = temp.path();
        let (pool, workspace_id) = fixture(data_root, &workspace_root);
        let repository = SqliteEffectRepository::new(pool);

        // 1-2. Configure (env reference only) → real reconcile (render → atomic write → activation
        //      → converge Gen 1). The fake renderer returns the real OpenCode shape carrying only
        //      the env reference; the host recomputes the digest so the marker vouches for it.
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);
        let coordinator = RecordingCoordinator::default();
        let request = claim(&repository, PUBLISHED_AT + 20);
        let outcome = reconcile_mcp_one(&repository, &coordinator, request.due, PUBLISHED_AT + 20);
        assert_eq!(
            outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1),
            },
            "the live smoke's main chain must converge through the real reconcile path",
        );
        assert!(
            repository
                .complete_reconcile_request(&request.claim, Generation::new(1), PUBLISHED_AT + 20)
                .unwrap()
        );

        // 3. The Ora-owned file carries ONLY the env reference, never the live key.
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        let file_contents = fs::read_to_string(&file).unwrap();
        assert!(
            !file_contents.contains(tavily_key.as_str()),
            "the Ora-owned config must carry only the env reference, never the live key",
        );
        assert!(
            file_contents.contains(&format!("{{env:{RENDERED_ENV_VAR}}}")),
            "the file must carry the env reference the live call resolves",
        );

        // 4. Ready: the Application State fold reads Ready only after convergence + a running Agent.
        let service = WorkspaceEffectService::new(repository);
        let response = service
            .mcp_application_state(
                GetMcpApplicationStateRequest {
                    workspace_id: workspace_id.to_string(),
                },
                /*agent_running*/ true,
            )
            .unwrap();
        assert_eq!(response.state, McpApplicationStateDto::Ready);

        // 5. Live MCP tool-call: resolve the {env:RENDERED_ENV_VAR} reference against the real key
        //    (as the OpenCode CLI would against the host-injected env) and call the live server.
        let activation_env = BTreeMap::from([(RENDERED_ENV_VAR.to_string(), tavily_key.clone())]);
        let authorization = activation_env
            .get(RENDERED_ENV_VAR)
            .map(|value| format!("Bearer {value}"))
            .expect("the activation binding carries the resolved key");
        let body = call_live_tavily_search(&authorization, "what is the capital of France").expect(
            "the live Tavily MCP server must accept the Bearer key and answer a tavily-search call",
        );
        assert!(
            !body.is_empty(),
            "the live tavily-search must return a non-empty result",
        );
        assert!(
            !body.contains(tavily_key.as_str()),
            "the live Tavily response must not echo the Authorization key",
        );

        // 6. No key leak into durable host state: the effect database carries only the env reference.
        let db_bytes = fs::read(data_root.join("ora.sqlite3")).unwrap();
        let needle = tavily_key.as_bytes();
        let leaked = db_bytes
            .windows(needle.len())
            .any(|window| window == needle);
        assert!(
            !leaked,
            "the live key must not persist anywhere in the effect database",
        );
    }

    /// Performs a real MCP `initialize` + `tools/call tavily-search` against the live Tavily server.
    ///
    /// Returns the `tools/call` response payload as JSON text (with any SSE `data:` framing
    /// stripped). The key lives only in the `Authorization` header; any error string is scrubbed of
    /// it (replaced with `[REDACTED]`) before it can reach the test output, so a transport failure
    /// cannot leak the credential through a diagnostic.
    fn call_live_tavily_search(authorization: &str, query: &str) -> Result<String, String> {
        let key = authorization
            .strip_prefix("Bearer ")
            .unwrap_or(authorization);
        let redact = |message: String| message.replace(key, "[REDACTED]");

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| redact(format!("failed to build HTTP client: {error}")))?;
        let endpoint = "https://mcp.tavily.com/mcp";
        let accept = "application/json, text/event-stream";

        // MCP initialize: establishes the session and proves the live server accepts the Bearer key.
        let init_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "ora-p1-smoke", "version": "0.0.0"}
            }
        });
        let init_resp = client
            .post(endpoint)
            .header("Authorization", authorization)
            .header("Accept", accept)
            .header("Content-Type", "application/json")
            .json(&init_body)
            .send()
            .map_err(|error| redact(format!("initialize request failed: {error}")))?;
        if !init_resp.status().is_success() {
            let status = init_resp.status();
            let text = init_resp.text().unwrap_or_default();
            return Err(redact(format!("initialize returned HTTP {status}: {text}")));
        }
        let session_id = init_resp
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let _init_text = init_resp
            .text()
            .map_err(|error| redact(format!("failed to read initialize body: {error}")))?;

        // notifications/initialized (best-effort; the server need not answer).
        let _ = client
            .post(endpoint)
            .header("Authorization", authorization)
            .header("Accept", accept)
            .header("Content-Type", "application/json")
            .header("mcp-session-id", session_id.as_deref().unwrap_or(""))
            .json(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .send();

        // tools/call tavily-search: the real tool invocation the OpenCode CLI would make.
        let call_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "tavily-search",
                "arguments": {"query": query, "max_results": 1}
            }
        });
        let mut call_req = client
            .post(endpoint)
            .header("Authorization", authorization)
            .header("Accept", accept)
            .header("Content-Type", "application/json")
            .json(&call_body);
        if let Some(session) = session_id.as_deref() {
            call_req = call_req.header("mcp-session-id", session);
        }
        let call_resp = call_req
            .send()
            .map_err(|error| redact(format!("tools/call request failed: {error}")))?;
        let status = call_resp.status();
        let call_text = call_resp
            .text()
            .map_err(|error| redact(format!("failed to read tools/call body: {error}")))?;
        if !status.is_success() {
            return Err(redact(format!(
                "tools/call returned HTTP {status}: {call_text}"
            )));
        }
        Ok(extract_jsonrpc_payload(&call_text))
    }

    /// Pulls the JSON-RPC result out of a response that may be plain JSON or SSE `data:` frames.
    fn extract_jsonrpc_payload(body: &str) -> String {
        for line in body.lines() {
            let trimmed = line.trim();
            let candidate = trimmed
                .strip_prefix("data:")
                .map(str::trim)
                .unwrap_or(trimmed);
            if candidate.starts_with('{') {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
                    if value.get("result").is_some() || value.get("error").is_some() {
                        return value.to_string();
                    }
                }
            }
        }
        body.to_string()
    }

    /// A surface whose file already proves the current generation reconciles as an idempotent no-op.
    ///
    /// Crash recovery is idempotent-forward: once the durable status says the generation was applied
    /// AND the file still carries the matching ownership marker, a later reconcile re-reads that proof
    /// and renders, writes, and coordinates nothing. This is what lets repeated wakeups coalesce
    /// exactly as they do for Skills, and lets a second pass over an already-current surface leave a
    /// live agent undisturbed.
    #[test]
    fn a_current_mcp_surface_reconciles_as_a_no_op_without_rendering_or_coordinating() {
        let temp = TempDir::new().unwrap();
        let workspace_root = temp.path().join("workspace");
        let (pool, workspace_id) = fixture(temp.path(), &workspace_root);
        let repository = SqliteEffectRepository::new(pool);
        select_tavily(&repository, &workspace_id, PUBLISHED_AT);
        declare_mcp_surface(&repository, &workspace_id, &workspace_root);

        // First pass renders and writes the file, advancing the surface to Current at generation 1.
        let first = claim(&repository, PUBLISHED_AT + 20);
        let second_due = first.due.clone();
        let first_outcome = reconcile_mcp_one(
            &repository,
            &RecordingCoordinator::default(),
            first.due,
            PUBLISHED_AT + 20,
        );
        assert_eq!(
            first_outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1)
            }
        );
        repository
            .complete_reconcile_request(&first.claim, Generation::new(1), PUBLISHED_AT + 20)
            .unwrap();
        let file = workspace_root.join(".opencode").join("opencode.jsonc");
        let written_before = fs::read_to_string(&file).unwrap();

        // A second pass over the same generation finds the durable status already applied and the file
        // still Ora-owned, so it converges without rendering, writing, or coordinating anything.
        let second = RecordingCoordinator::default();
        let second_outcome = reconcile_mcp_one(&repository, &second, second_due, PUBLISHED_AT + 30);
        assert_eq!(
            second_outcome,
            SurfaceOutcome::Converged {
                generation: Generation::new(1)
            },
            "an already-current surface must converge without redoing the work",
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            written_before,
            "an idempotent no-op must not rewrite the file it already proved",
        );
        assert!(
            second
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty(),
            "an already-current surface must not quiesce, render, or restart the consumer",
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
            ..Default::default()
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
