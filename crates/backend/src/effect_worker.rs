//! Drives durable Generic Target requests through the Effect reconciler.

use crate::agent_runtime::{ReplacedAgentSessions, plugin_agent};
use crate::clock::SystemClock;
use crate::effect_registration::converge_workspace_targets;
use crate::plugin::PluginApi;
use crate::session_setup::BarrierReason;
use ora_application::Clock;
use ora_db::{RepositoryPool, SqliteEffectRepository, SqliteWorkspaceRepository};
use ora_domain::PluginId;
use ora_effect::{
    AdapterReceipt, ConsumerAdapter, ConsumerAdapterError, CoordinationContract, CoordinationPlan,
    CoordinationReceipt, CoordinationReceiptState, EffectReconciler, EffectRepository,
    EffectTarget, EffectTargetId, LocalTimestamp, ReadinessReceipt, ReconcileOutcome,
    TargetProjection, WorkerIdentity,
};
use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
use ora_logging::{ora_info, ora_warn};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;
use tokio::runtime::Handle;
use uuid::Uuid;

const SCAN_INTERVAL: Duration = Duration::from_secs(30);
const TARGET_BATCH_SIZE: usize = 16;
const LEASE_DURATION: Duration = Duration::from_secs(300);
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Coalesced in-process hint; SQLite remains the durable source of owed work.
#[derive(Debug, Default)]
struct WakeSignal {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl WakeSignal {
    /// Requests one worker pass while coalescing concurrent hints.
    fn notify(&self) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_one();
    }

    /// Waits for either a hint or the periodic level-triggered scan.
    fn wait(&self, timeout: Duration) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if !*pending {
            let (guard, _) = self
                .changed
                .wait_timeout(pending, timeout)
                .unwrap_or_else(PoisonError::into_inner);
            pending = guard;
        }
        *pending = false;
    }
}

/// Wakes the Effect worker after an already-durable Desired or declaration transition.
#[derive(Clone, Debug)]
pub(crate) struct EffectWorkerHandle {
    wake: Arc<WakeSignal>,
}

impl EffectWorkerHandle {
    pub(crate) fn notify(&self) {
        self.wake.notify();
    }
}

/// Claims and reconciles Target obligations without retaining a second state machine in memory.
pub(crate) struct EffectWorker<Sessions> {
    repository: SqliteEffectRepository,
    workspace_repository: SqliteWorkspaceRepository,
    plugin_host: Arc<PluginApi>,
    sessions: Arc<Sessions>,
    clock: SystemClock,
    wake: Arc<WakeSignal>,
    worker: WorkerIdentity,
}

impl<Sessions: ReplacedAgentSessions> EffectWorker<Sessions> {
    pub(crate) fn new(
        pool: RepositoryPool,
        plugin_host: Arc<PluginApi>,
        sessions: Arc<Sessions>,
    ) -> Self {
        let worker = WorkerIdentity::parse(Uuid::new_v4().to_string()).unwrap_or_else(|error| {
            unreachable!("UUID worker identity is always non-empty: {error}")
        });
        Self {
            repository: SqliteEffectRepository::new(pool.clone()),
            workspace_repository: SqliteWorkspaceRepository::new(pool),
            plugin_host,
            sessions,
            clock: SystemClock,
            wake: Arc::new(WakeSignal::default()),
            worker,
        }
    }

    /// Hands out a latency-only wake handle before the worker moves onto its thread.
    pub(crate) fn handle(&self) -> EffectWorkerHandle {
        EffectWorkerHandle {
            wake: self.wake.clone(),
        }
    }

    /// Runs blocking filesystem reconciliation on a dedicated thread with local async IPC.
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
                            "failed to build Effect worker runtime",
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
            ora_warn!(
                operation = "effect_reconcile",
                error = %error,
                "failed to spawn Effect worker; durable requests remain queued",
            );
        }
        handle
    }

    /// Converts ambiguous startup journals into explicit manual recovery before workers may claim.
    pub(crate) fn recover(&self) {
        let detected_at = LocalTimestamp::from_millis(self.clock.now_timestamp_millis());
        match self
            .repository
            .quarantine_unfinished_operations(detected_at)
        {
            Ok(0) => {}
            Ok(operation_count) => ora_warn!(
                operation = "effect_recovery",
                unfinished_operations = operation_count,
                "unfinished Effect operations were quarantined for explicit recovery",
            ),
            Err(error) => ora_warn!(
                operation = "effect_recovery",
                error = %error,
                "failed to inspect unfinished Effect operations",
            ),
        }
    }

    /// Re-pairs current declarations, claims one fair batch, and reconciles each independently.
    pub(crate) fn run_pass(&self, runtime: &Handle) {
        let now = self.clock.now_timestamp_millis();
        self.converge_target_declarations(now);
        let now = LocalTimestamp::from_millis(now);
        if let Err(error) = self.repository.quarantine_unfinished_operations(now) {
            ora_warn!(
                operation = "effect_recovery",
                error = %error,
                "failed to quarantine unfinished Effect operations; Target claiming is paused",
            );
            return;
        }
        let lease_until =
            LocalTimestamp::from_millis(now.millis() + LEASE_DURATION.as_millis() as i64);
        let claimed = match self.repository.claim_due_targets(
            &self.worker,
            now,
            lease_until,
            TARGET_BATCH_SIZE,
        ) {
            Ok(claimed) => claimed,
            Err(error) => {
                ora_warn!(
                    operation = "effect_reconcile",
                    error = %error,
                    "failed to claim due Effect Targets",
                );
                return;
            }
        };
        for (target, claim) in claimed {
            self.reconcile_target(runtime, &target, &claim, now, lease_until);
        }
    }

    /// Closes Workspace/declaration ordering gaps before requests are claimed.
    fn converge_target_declarations(&self, now: i64) {
        let declarations = self.plugin_host.agent_effect_declarations();
        if declarations.is_empty() {
            return;
        }
        let workspaces = match self.workspace_repository.list_all_workspaces() {
            Ok(workspaces) => workspaces,
            Err(error) => {
                ora_warn!(
                    operation = "effect_declaration",
                    error = %error,
                    "failed to list Workspaces for Effect Target convergence",
                );
                return;
            }
        };
        if let Err(error) =
            converge_workspace_targets(&self.repository, &workspaces, &declarations, now)
        {
            ora_warn!(
                operation = "effect_declaration",
                error = %error,
                "failed to converge Effect Target declarations",
            );
        }
    }

    /// Delegates one claimed Target to the deep reconciler and records only an operational log.
    fn reconcile_target(
        &self,
        runtime: &Handle,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
    ) {
        let planner = SkillPlanner;
        let resource_adapter = SkillDirectoryResourceAdapter;
        let consumer_adapter = PluginConsumerAdapter {
            plugin_host: self.plugin_host.as_ref(),
            sessions: self.sessions.as_ref(),
            runtime,
            coordinated: Mutex::new(BTreeSet::new()),
            held_barriers: Mutex::new(HashMap::new()),
        };
        let reconciler = EffectReconciler::new(
            &self.repository,
            &planner,
            &consumer_adapter,
            &resource_adapter,
        );
        match reconciler.reconcile(target, claim, now, lease_until) {
            Ok(ReconcileOutcome::Current { generation, .. }) => ora_info!(
                operation = "effect_reconcile",
                target = target.as_str(),
                generation = generation.value(),
                "Effect Target is current",
            ),
            Ok(ReconcileOutcome::Blocked { generation, .. }) => ora_info!(
                operation = "effect_reconcile",
                target = target.as_str(),
                generation = generation.value(),
                "Effect Target is blocked by structured Conditions",
            ),
            Ok(ReconcileOutcome::Mutated {
                generation,
                operations,
                ..
            }) => ora_info!(
                operation = "effect_reconcile",
                target = target.as_str(),
                generation = generation.value(),
                operations,
                "Effect Target Resources were verified and finalized",
            ),
            Err(error) => {
                ora_warn!(
                    operation = "effect_reconcile",
                    target = target.as_str(),
                    error = %error,
                    "Effect Target reconcile failed",
                );
                if let Err(recovery_error) = self.repository.quarantine_unfinished_operations(now) {
                    ora_warn!(
                        operation = "effect_recovery",
                        target = target.as_str(),
                        error = %recovery_error,
                        "failed to quarantine a possibly unfinished Effect operation",
                    );
                    return;
                }
                let not_before =
                    LocalTimestamp::from_millis(now.millis() + RETRY_DELAY.as_millis() as i64);
                if let Err(retry_error) = self
                    .repository
                    .schedule_retry(target, claim, not_before, now)
                {
                    ora_warn!(
                        operation = "effect_retry",
                        target = target.as_str(),
                        error = %retry_error,
                        "failed to schedule the Effect Target retry",
                    );
                }
            }
        }
    }
}

/// Consumer adapter that keeps plugin IPC and session repair outside Effect Core.
struct PluginConsumerAdapter<'a, Sessions> {
    plugin_host: &'a PluginApi,
    sessions: &'a Sessions,
    runtime: &'a Handle,
    coordinated: Mutex<BTreeSet<EffectTargetId>>,
    held_barriers: Mutex<HashMap<PluginId, crate::session_setup::BarrierGuard>>,
}

impl<Sessions> PluginConsumerAdapter<'_, Sessions> {
    /// Resolves only a currently running plugin generation; disconnected Consumers are already safe.
    fn running_runtime(
        &self,
        target: &EffectTarget,
    ) -> Result<Option<ora_plugin_runtime::PluginRuntime>, ConsumerAdapterError> {
        let plugin_id =
            PluginId::parse(&target.consumer.stable_key).map_err(ConsumerAdapterError::new)?;
        Ok(self
            .plugin_host
            .lifecycle
            .connection(&plugin_id)
            .ok()
            .map(|connection| connection.runtime().process().clone()))
    }

    /// Returns the exact versioned coordination contract selected for this Target.
    fn coordination_contract(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationContract, ConsumerAdapterError> {
        match plan.participants.get(&target.identity) {
            Some(ora_effect::CoordinationRequirement::QuiesceBeforeMutation(contract)) => {
                Ok(contract.clone())
            }
            Some(ora_effect::CoordinationRequirement::Uninterrupted) | None => {
                Err(ConsumerAdapterError::new(std::io::Error::other(
                    "coordination was requested without a quiesce contract",
                )))
            }
        }
    }
}

impl<Sessions: ReplacedAgentSessions> ConsumerAdapter for PluginConsumerAdapter<'_, Sessions> {
    fn coordinate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        let contract = self.coordination_contract(target, plan)?;
        let plugin_id =
            PluginId::parse(&target.consumer.stable_key).map_err(ConsumerAdapterError::new)?;
        let barrier = self.runtime.block_on(
            self.sessions
                .session_barriers()
                .for_plugin(&plugin_id)
                .acquire(BarrierReason::EffectMutation),
        );
        self.held_barriers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(plugin_id, barrier);
        let proof = match self.running_runtime(target)? {
            Some(runtime) => {
                let receipt = self
                    .runtime
                    .block_on(plugin_agent::coordinate(&runtime, target, plan))
                    .map_err(ConsumerAdapterError::new)?;
                self.coordinated
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(target.identity.clone());
                receipt
            }
            None => disconnected_receipt(),
        };
        Ok(CoordinationReceipt {
            target: target.identity.clone(),
            contract,
            state: CoordinationReceiptState::SafeToMutate,
            proof,
        })
    }

    fn reactivate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        let contract = self.coordination_contract(target, plan)?;
        let runtime = self.running_runtime(target)?;
        let proof = match runtime {
            Some(runtime) => self
                .runtime
                .block_on(plugin_agent::reactivate(&runtime, target, plan))
                .map_err(ConsumerAdapterError::new)?,
            None => disconnected_receipt(),
        };
        let was_coordinated = self
            .coordinated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&target.identity);
        let plugin_id =
            PluginId::parse(&target.consumer.stable_key).map_err(ConsumerAdapterError::new)?;
        drop(
            self.held_barriers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&plugin_id),
        );
        if was_coordinated {
            self.sessions
                .detach_sessions_for_replaced_plugin(&plugin_id);
        }
        Ok(CoordinationReceipt {
            target: target.identity.clone(),
            contract,
            state: CoordinationReceiptState::Reactivated,
            proof,
        })
    }

    fn verify_ready(
        &self,
        target: &EffectTarget,
        projection: &TargetProjection,
    ) -> Result<ReadinessReceipt, ConsumerAdapterError> {
        let proof = match self.running_runtime(target)? {
            Some(runtime) => self
                .runtime
                .block_on(plugin_agent::verify_ready(&runtime, target, projection))
                .map_err(ConsumerAdapterError::new)?,
            None => disconnected_receipt(),
        };
        Ok(ReadinessReceipt {
            target: target.identity.clone(),
            generation: projection.generation,
            consumer_revision: projection.consumer_revision.clone(),
            projection: projection.digest.clone(),
            proof,
        })
    }
}

/// Records why no IPC proof was needed without treating disconnection as a core phase.
fn disconnected_receipt() -> AdapterReceipt {
    AdapterReceipt {
        version: 1,
        payload: json!({ "consumerState": "disconnected" }),
    }
}
