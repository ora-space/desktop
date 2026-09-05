use crate::{
    ArtifactState, AttemptFinalization, ConditionGeneration, ConditionImpact, ConditionOwner,
    ConditionProposal, ConditionRetry, ConditionSubject, ConsumerAdapter, ConsumerAdapterError,
    CoordinationPlan, CoordinationReceipt, CoordinationReceiptState, CoordinationRequirement,
    EffectOperationId, EffectPlanner, EffectRepository, EffectResourceId, EffectTargetId,
    Generation, LocalTimestamp, ManagedItem, PlanningResult, ProjectionCommit, ReconcileAttempt,
    ReconcileAttemptId, ReconcileAttemptIntent, ReconcileClaim, RepositoryError, ResourceAdapter,
    ResourceAdapterError, ResourcePlan, ResourcePlanningInput, SafeConditionDetails,
    StableConditionCode, StatusTransitionError, TargetIssueState, TargetPlanningInput,
};
use ora_utils::clock::TimestampSource;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Observable result of one claimed Target reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconcileOutcome {
    Current {
        target: EffectTargetId,
        generation: Generation,
    },
    Blocked {
        target: EffectTargetId,
        generation: Generation,
        conditions: Vec<ConditionProposal>,
    },
    Mutated {
        target: EffectTargetId,
        generation: Generation,
        operations: usize,
    },
}

/// Durable Generic Target reconciler with statically dispatched planners and adapters.
pub struct EffectReconciler<'a, Repository, Planner, Consumer, Resource, Clock> {
    repository: &'a Repository,
    planner: &'a Planner,
    consumer_adapter: &'a Consumer,
    resource_adapter: &'a Resource,
    clock: &'a Clock,
}

/// Complete mutation-path inputs grouped so the transition cannot receive mismatched fragments.
struct MutationPass {
    snapshot: crate::ReconcileSnapshot,
    target_projection: crate::TargetProjection,
    contributor_projections: BTreeMap<EffectTargetId, crate::TargetProjection>,
    resource_plans: BTreeMap<EffectResourceId, ResourcePlan>,
}

/// Closed planning result keeps blocking facts separate from a complete mutation candidate.
enum SnapshotPlan {
    Ready(MutationPass),
    Blocked {
        snapshot: crate::ReconcileSnapshot,
        conditions: Vec<ConditionProposal>,
    },
}

impl<'a, Repository, Planner, Consumer, Resource, Clock>
    EffectReconciler<'a, Repository, Planner, Consumer, Resource, Clock>
where
    Repository: EffectRepository,
    Planner: EffectPlanner,
    Consumer: ConsumerAdapter,
    Resource: ResourceAdapter,
    Clock: TimestampSource,
{
    pub fn new(
        repository: &'a Repository,
        planner: &'a Planner,
        consumer_adapter: &'a Consumer,
        resource_adapter: &'a Resource,
        clock: &'a Clock,
    ) -> Self {
        Self {
            repository,
            planner,
            consumer_adapter,
            resource_adapter,
            clock,
        }
    }

    /// Reconciles a claimed Target from freshly reloaded facts through exact readiness proof.
    pub fn reconcile(
        &self,
        target_id: &EffectTargetId,
        claim: &ReconcileClaim,
        resource_lease_until: LocalTimestamp,
    ) -> Result<ReconcileOutcome, ReconcileError> {
        let now = LocalTimestamp::from_millis(self.clock.current_timestamp_millis());
        let mut snapshot = self.repository.load_reconcile_snapshot(target_id, claim)?;
        let resource_ids = snapshot
            .declaration
            .bindings
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if !resource_ids.is_empty() {
            let resource_claims = self
                .repository
                .claim_resources(
                    &snapshot.target.identity,
                    claim,
                    &resource_ids,
                    now,
                    resource_lease_until,
                )?
                .ok_or(ReconcileError::ResourceClaimUnavailable)?;
            if resource_claims.len() != resource_ids.len() {
                return Err(ReconcileError::ResourceClaimUnavailable);
            }

            // Only observations made after Resource fencing can support ResourceStatus or
            // readiness. A declaration change during acquisition is retried as a new snapshot.
            snapshot = self.repository.load_reconcile_snapshot(target_id, claim)?;
            let refreshed_resource_ids = snapshot
                .declaration
                .bindings
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            if refreshed_resource_ids != resource_ids {
                return Err(ReconcileError::ClaimedResourceSetChanged);
            }
        }
        let pass = match self.plan_snapshot(snapshot)? {
            SnapshotPlan::Ready(pass) => pass,
            SnapshotPlan::Blocked {
                snapshot,
                conditions,
            } => return self.commit_blocked(snapshot, conditions, claim),
        };
        let mutation_count = pass
            .resource_plans
            .values()
            .flat_map(|plan| &plan.changes)
            .filter(|change| matches!(change, crate::PlannedResourceChange::Mutate(_)))
            .count();
        if mutation_count == 0 {
            return self.commit_current_without_mutation(
                pass.snapshot,
                pass.target_projection,
                pass.contributor_projections,
                pass.resource_plans,
                claim,
            );
        }
        self.apply_mutations(pass, claim)
    }

    /// Plans one complete snapshot, including all contributors to each shared Resource.
    fn plan_snapshot(
        &self,
        mut snapshot: crate::ReconcileSnapshot,
    ) -> Result<SnapshotPlan, ReconcileError> {
        let generation = snapshot.desired.generation;
        if snapshot.target_status.progress().desired() < generation {
            snapshot.target_status.request_generation(generation)?;
        }
        let target_projection = match self.planner.project_target(TargetPlanningInput {
            desired: &snapshot.desired,
            target: &snapshot.target,
            consumer_revision: &snapshot.consumer_revision,
            declaration: &snapshot.declaration,
            resources: &snapshot.resources,
            revisions: &snapshot.revisions,
        })? {
            PlanningResult::Projected(projection) => projection,
            PlanningResult::Blocked(conditions) => {
                return Ok(SnapshotPlan::Blocked {
                    snapshot,
                    conditions,
                });
            }
        };
        snapshot.target_status.record_observed(generation)?;

        let mut all_conditions = Vec::new();
        let mut contributor_projections =
            BTreeMap::from([(target_projection.target.clone(), target_projection.clone())]);
        let mut requirements_by_resource = BTreeMap::new();
        for resource_id in target_projection.resource_requirements.keys() {
            let mut requirements = Vec::new();
            for related in snapshot.related_targets.values() {
                if !related.declaration.bindings.contains_key(resource_id) {
                    continue;
                }
                let related_projection = if related.target.identity == target_projection.target {
                    target_projection.clone()
                } else {
                    match self.planner.project_target(TargetPlanningInput {
                        desired: &snapshot.desired,
                        target: &related.target,
                        consumer_revision: &related.consumer_revision,
                        declaration: &related.declaration,
                        resources: &snapshot.resources,
                        revisions: &snapshot.revisions,
                    })? {
                        PlanningResult::Projected(projection) => projection,
                        PlanningResult::Blocked(mut conditions) => {
                            // This reconcile may explain a contributor failure but cannot mutate
                            // another Target's Condition set without that Target's own claim.
                            for condition in &mut conditions {
                                condition.owner =
                                    ConditionOwner::Target(snapshot.target.identity.clone());
                            }
                            all_conditions.extend(conditions);
                            continue;
                        }
                    }
                };
                contributor_projections.insert(
                    related_projection.target.clone(),
                    related_projection.clone(),
                );
                if let Some(requirement) = related_projection.resource_requirements.get(resource_id)
                {
                    requirements.push(requirement.clone());
                }
            }
            requirements_by_resource.insert(resource_id.clone(), requirements);
        }

        let mut resource_plans = BTreeMap::new();
        for resource_id in target_projection.resource_requirements.keys() {
            let resource = snapshot
                .resources
                .get(resource_id)
                .ok_or_else(|| ReconcileError::ResourceMissing(resource_id.clone()))?;
            let observation = self.resource_adapter.observe(resource)?;
            let status = snapshot
                .resource_statuses
                .get_mut(resource_id)
                .ok_or_else(|| ReconcileError::ResourceStatusMissing(resource_id.clone()))?;
            if status.desired() < generation {
                status.request_generation(generation)?;
            }
            status.record_observed(generation)?;
            let requirements = requirements_by_resource
                .get(resource_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let managed = snapshot
                .managed
                .get(resource_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            match self.planner.plan_resource(ResourcePlanningInput {
                resource,
                generation,
                requirements,
                desired_effects: &snapshot.desired.effects,
                revisions: &snapshot.revisions,
                managed,
                observed: &observation,
            })? {
                PlanningResult::Projected(plan) => {
                    resource_plans.insert(resource_id.clone(), plan);
                }
                PlanningResult::Blocked(conditions) => all_conditions.extend(conditions),
            }
        }
        if !all_conditions.is_empty() {
            return Ok(SnapshotPlan::Blocked {
                snapshot,
                conditions: all_conditions,
            });
        }
        Ok(SnapshotPlan::Ready(MutationPass {
            snapshot,
            target_projection,
            contributor_projections,
            resource_plans,
        }))
    }

    /// Persists planner Conditions and releases the claimed request into its blocked state.
    fn commit_blocked(
        &self,
        snapshot: crate::ReconcileSnapshot,
        conditions: Vec<ConditionProposal>,
        claim: &ReconcileClaim,
    ) -> Result<ReconcileOutcome, ReconcileError> {
        let target = snapshot.target.identity.clone();
        let generation = snapshot.desired.generation;
        self.repository.block_target(
            &target,
            claim,
            snapshot.target_status,
            snapshot.resource_statuses.into_values().collect(),
            conditions.clone(),
        )?;
        Ok(ReconcileOutcome::Blocked {
            target,
            generation,
            conditions,
        })
    }

    /// Commits verified Resource/Target watermarks when projection already matches observation.
    fn commit_current_without_mutation(
        &self,
        mut snapshot: crate::ReconcileSnapshot,
        target_projection: crate::TargetProjection,
        contributor_projections: BTreeMap<EffectTargetId, crate::TargetProjection>,
        resource_plans: BTreeMap<EffectResourceId, ResourcePlan>,
        claim: &ReconcileClaim,
    ) -> Result<ReconcileOutcome, ReconcileError> {
        let generation = target_projection.generation;
        for resource_id in resource_plans.keys() {
            snapshot
                .resource_statuses
                .get_mut(resource_id)
                .ok_or_else(|| ReconcileError::ResourceStatusMissing(resource_id.clone()))?
                .record_applied(generation)?;
        }
        snapshot.target_status.record_applied(generation)?;
        let readiness = self
            .consumer_adapter
            .verify_ready(&snapshot.target, &target_projection)?;
        snapshot.target_status.record_ready(
            &readiness,
            &snapshot.consumer_revision.identity,
            &target_projection.digest,
            TargetIssueState::Clear,
        )?;
        let managed = projected_managed(&resource_plans, generation);
        let projected_identities = managed
            .iter()
            .map(|item| item.identity.clone())
            .collect::<BTreeSet<_>>();
        let removed_managed = snapshot
            .managed
            .into_values()
            .flatten()
            .filter(|item| !projected_identities.contains(&item.identity))
            .map(|item| item.identity)
            .collect();
        self.repository.commit_projection(
            claim,
            ProjectionCommit {
                target_projections: contributor_projections.into_values().collect(),
                resource_projections: resource_plans
                    .into_values()
                    .map(|plan| plan.projection)
                    .collect(),
                target_status: snapshot.target_status,
                resource_statuses: snapshot.resource_statuses.into_values().collect(),
                managed,
                removed_managed,
                conditions: Vec::new(),
                readiness: Some(readiness),
            },
        )?;
        Ok(ReconcileOutcome::Current {
            target: snapshot.target.identity,
            generation,
        })
    }

    /// Journals and applies a plan that was observed while holding every Resource claim.
    fn apply_mutations(
        &self,
        pass: MutationPass,
        claim: &ReconcileClaim,
    ) -> Result<ReconcileOutcome, ReconcileError> {
        let MutationPass {
            mut snapshot,
            target_projection,
            contributor_projections,
            resource_plans,
        } = pass;
        let prepared_at = LocalTimestamp::from_millis(self.clock.current_timestamp_millis());
        let generation = target_projection.generation;

        let resource_ids = resource_plans
            .iter()
            .filter(|(_, plan)| {
                plan.changes
                    .iter()
                    .any(|change| matches!(change, crate::PlannedResourceChange::Mutate(_)))
            })
            .map(|(resource, _)| resource.clone())
            .collect::<Vec<_>>();
        let coordination = coordination_plan(&snapshot, &resource_ids)?;
        let attempt_id = ReconcileAttemptId::random();
        let mut operations = Vec::new();
        let mut artifacts = Vec::new();
        let mut removed_managed = Vec::new();
        let mut sequence = 0_u32;
        for (resource_id, plan) in &resource_plans {
            let resource = snapshot
                .resources
                .get(resource_id)
                .ok_or_else(|| ReconcileError::ResourceMissing(resource_id.clone()))?;
            for change in &plan.changes {
                match change {
                    crate::PlannedResourceChange::Mutate(mutation) => {
                        let prepared = self.resource_adapter.prepare_operation(
                            resource,
                            attempt_id.clone(),
                            generation,
                            sequence,
                            mutation.as_ref().clone(),
                            prepared_at,
                        )?;
                        sequence = sequence
                            .checked_add(1)
                            .ok_or(ReconcileError::OperationSequenceExhausted)?;
                        operations.push(prepared.operation);
                        artifacts.extend(prepared.artifacts);
                    }
                    crate::PlannedResourceChange::ForgetMissing(identity) => {
                        removed_managed.push(identity.clone());
                    }
                }
            }
        }
        let mut attempt = ReconcileAttempt::prepare(
            attempt_id,
            ReconcileAttemptIntent {
                target: snapshot.target.identity.clone(),
                generation,
                consumer_revision: snapshot.consumer_revision.identity.clone(),
                target_projection: target_projection.digest.clone(),
                resource_projections: resource_plans
                    .values()
                    .map(|plan| plan.projection.digest.clone())
                    .collect(),
                coordination: coordination.clone(),
                operations: operations
                    .iter()
                    .map(|operation| operation.identity().clone())
                    .collect(),
            },
        )?;
        self.repository.prepare_attempt(
            claim,
            attempt.clone(),
            contributor_projections.into_values().collect(),
            resource_plans
                .values()
                .map(|plan| plan.projection.clone())
                .collect(),
            operations.clone(),
            artifacts.clone(),
        )?;

        let mut receipts = Vec::new();
        let mut coordinated_targets = BTreeSet::new();
        for (participant_id, requirement) in &coordination.participants {
            if matches!(requirement, CoordinationRequirement::Uninterrupted) {
                continue;
            }
            let participant = snapshot
                .participant_targets
                .get(participant_id)
                .ok_or_else(|| ReconcileError::ParticipantMissing(participant_id.clone()))?;
            let receipt = self
                .consumer_adapter
                .coordinate(participant, &coordination)?;
            validate_coordination_receipt(
                &receipt,
                participant_id,
                requirement,
                CoordinationReceiptState::SafeToMutate,
            )?;
            receipts.push(receipt);
            coordinated_targets.insert(participant_id.clone());
            self.repository
                .record_attempt_progress(claim, &attempt, &operations, &receipts)?;
        }
        attempt.mark_coordinated()?;
        self.repository
            .record_attempt_progress(claim, &attempt, &operations, &receipts)?;

        for operation_index in 0..operations.len() {
            let apply_receipt = self.resource_adapter.apply(&operations[operation_index])?;
            if apply_receipt.operation != *operations[operation_index].identity() {
                return Err(ReconcileError::MismatchedOperationReceipt(
                    operations[operation_index].identity().clone(),
                ));
            }
            operations[operation_index].mark_applied(LocalTimestamp::from_millis(
                self.clock.current_timestamp_millis(),
            ))?;
            self.repository
                .record_attempt_progress(claim, &attempt, &operations, &receipts)?;
            let verification = self.resource_adapter.verify(&operations[operation_index])?;
            if verification.operation != *operations[operation_index].identity() {
                return Err(ReconcileError::MismatchedOperationReceipt(
                    operations[operation_index].identity().clone(),
                ));
            }
        }
        attempt.mark_applied()?;
        self.repository
            .record_attempt_progress(claim, &attempt, &operations, &receipts)?;
        attempt.mark_verified()?;
        self.repository
            .record_attempt_progress(claim, &attempt, &operations, &receipts)?;

        for participant_id in coordinated_targets {
            let participant = snapshot
                .participant_targets
                .get(&participant_id)
                .ok_or_else(|| ReconcileError::ParticipantMissing(participant_id.clone()))?;
            let receipt = self
                .consumer_adapter
                .reactivate(participant, &coordination)?;
            let requirement = coordination
                .participants
                .get(&participant_id)
                .ok_or_else(|| ReconcileError::ParticipantMissing(participant_id.clone()))?;
            validate_coordination_receipt(
                &receipt,
                &participant_id,
                requirement,
                CoordinationReceiptState::Reactivated,
            )?;
            receipts.push(receipt);
        }
        // Reactivation is an all-participant barrier. Persisting a partial receipt set while the
        // Attempt is still Verified would violate the repository's exact barrier invariant.
        attempt.mark_activated()?;
        self.repository
            .record_attempt_progress(claim, &attempt, &operations, &receipts)?;

        for resource_id in resource_plans.keys() {
            snapshot
                .resource_statuses
                .get_mut(resource_id)
                .ok_or_else(|| ReconcileError::ResourceStatusMissing(resource_id.clone()))?
                .record_applied(generation)?;
        }
        snapshot.target_status.record_applied(generation)?;
        let readiness = self
            .consumer_adapter
            .verify_ready(&snapshot.target, &target_projection)?;
        snapshot.target_status.record_ready(
            &readiness,
            &snapshot.consumer_revision.identity,
            &target_projection.digest,
            TargetIssueState::Clear,
        )?;
        let managed = projected_managed(&resource_plans, generation);
        let projected_identities = managed
            .iter()
            .map(|item| item.identity.clone())
            .collect::<BTreeSet<_>>();
        for current in snapshot.managed.into_values().flatten() {
            if !projected_identities.contains(&current.identity) {
                removed_managed.push(current.identity);
            }
        }
        removed_managed.sort();
        removed_managed.dedup();
        for operation in &mut operations {
            operation.finalize(LocalTimestamp::from_millis(
                self.clock.current_timestamp_millis(),
            ))?;
        }
        attempt.finalize()?;
        self.repository.finalize_attempt(
            claim,
            AttemptFinalization {
                attempt,
                operations,
                managed,
                removed_managed,
                target_statuses: vec![snapshot.target_status],
                resource_statuses: snapshot.resource_statuses.into_values().collect(),
                readiness: Some(readiness),
                coordination_receipts: receipts,
                conditions: Vec::new(),
            },
        )?;
        for mut artifact in artifacts {
            artifact.state = ArtifactState::PendingCleanup;
            match self.resource_adapter.cleanup(&artifact) {
                Ok(receipt) => self
                    .repository
                    .complete_artifact_cleanup(&artifact.identity, receipt)?,
                Err(_) => {
                    artifact.state = ArtifactState::CleanupFailed;
                    self.repository.mark_artifact_cleanup_failed(artifact)?;
                }
            }
        }
        Ok(ReconcileOutcome::Mutated {
            target: snapshot.target.identity,
            generation,
            operations: sequence as usize,
        })
    }
}

/// Rejects Consumer acknowledgements that do not prove the exact requested coordination step.
fn validate_coordination_receipt(
    receipt: &CoordinationReceipt,
    target: &EffectTargetId,
    requirement: &CoordinationRequirement,
    expected_state: CoordinationReceiptState,
) -> Result<(), ReconcileError> {
    let CoordinationRequirement::QuiesceBeforeMutation(contract) = requirement else {
        return Err(ReconcileError::MismatchedCoordinationReceipt(
            target.clone(),
        ));
    };
    if receipt.target != *target || receipt.contract != *contract || receipt.state != expected_state
    {
        return Err(ReconcileError::MismatchedCoordinationReceipt(
            target.clone(),
        ));
    }
    Ok(())
}

/// Builds the union of every affected binding, preserving Uninterrupted participants explicitly.
fn coordination_plan(
    snapshot: &crate::ReconcileSnapshot,
    resources: &[EffectResourceId],
) -> Result<CoordinationPlan, ReconcileError> {
    let mut participants = BTreeMap::new();
    for resource in resources {
        let resource_participants = snapshot
            .coordination_participants
            .get(resource)
            .ok_or_else(|| ReconcileError::CoordinationParticipantsMissing(resource.clone()))?;
        for (target, requirement) in resource_participants {
            match (participants.get(target), requirement) {
                (
                    Some(CoordinationRequirement::QuiesceBeforeMutation(existing)),
                    CoordinationRequirement::QuiesceBeforeMutation(next),
                ) if existing != next => {
                    return Err(ReconcileError::CoordinationContractConflict(target.clone()));
                }
                (
                    Some(CoordinationRequirement::QuiesceBeforeMutation(_)),
                    CoordinationRequirement::Uninterrupted,
                ) => {}
                _ => {
                    participants.insert(target.clone(), requirement.clone());
                }
            }
        }
    }
    CoordinationPlan::new(resources.iter().cloned().collect(), participants)
        .map_err(ReconcileError::Operation)
}

/// Produces the complete post-verification ledger from every Resource projection.
fn projected_managed(
    plans: &BTreeMap<EffectResourceId, ResourcePlan>,
    generation: Generation,
) -> Vec<ManagedItem> {
    plans
        .iter()
        .flat_map(|(resource, plan)| {
            plan.projection
                .items
                .values()
                .map(move |materialization| ManagedItem {
                    identity: materialization.managed_identity.clone(),
                    resource: resource.clone(),
                    desired_effect: materialization.desired_effect.clone(),
                    applied_revision: materialization.revision.clone(),
                    native_identity: materialization.native_identity.clone(),
                    fingerprint: materialization.fingerprint.clone(),
                    applied_generation: generation,
                })
        })
        .collect()
}

/// Builds a safe recovery Condition for an operation that cannot be proven automatically.
pub fn recovery_condition(
    target: &EffectTargetId,
    operation: &crate::EffectOperationId,
    generation: Generation,
) -> ConditionProposal {
    ConditionProposal {
        owner: ConditionOwner::Target(target.clone()),
        subject: ConditionSubject::Operation(operation.clone()),
        code: StableConditionCode::from_static("recovery_required"),
        impact: ConditionImpact::Blocking,
        retry: ConditionRetry::Manual,
        generation: ConditionGeneration::At(generation),
        safe_details: SafeConditionDetails {
            message: "An Effect operation requires explicit recovery.".to_string(),
            parameters: BTreeMap::new(),
        },
    }
}

/// Builds the Resource-owned counterpart of a manual operation recovery Condition.
pub fn resource_recovery_condition(
    resource: &EffectResourceId,
    operation: &crate::EffectOperationId,
    generation: Generation,
) -> ConditionProposal {
    ConditionProposal {
        owner: ConditionOwner::Resource(resource.clone()),
        subject: ConditionSubject::Operation(operation.clone()),
        code: StableConditionCode::from_static("recovery_required"),
        impact: ConditionImpact::Blocking,
        retry: ConditionRetry::Manual,
        generation: ConditionGeneration::At(generation),
        safe_details: SafeConditionDetails {
            message: "An Effect operation requires explicit recovery.".to_string(),
            parameters: BTreeMap::new(),
        },
    }
}

/// Reports failure before or during one durable Target reconcile.
#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Planner(#[from] crate::PlannerError),
    #[error(transparent)]
    Consumer(#[from] ConsumerAdapterError),
    #[error(transparent)]
    Resource(#[from] ResourceAdapterError),
    #[error(transparent)]
    Status(#[from] StatusTransitionError),
    #[error(transparent)]
    Operation(#[from] crate::OperationTransitionError),
    #[error("Effect Resource {0} is missing from the claimed snapshot")]
    ResourceMissing(EffectResourceId),
    #[error("Effect Resource {0} has no status in the claimed snapshot")]
    ResourceStatusMissing(EffectResourceId),
    #[error("Effect Resource claims could not be acquired")]
    ResourceClaimUnavailable,
    #[error("the Target Resource set changed while acquiring mutation authority")]
    ClaimedResourceSetChanged,
    #[error("coordination participants are missing for Resource {0}")]
    CoordinationParticipantsMissing(EffectResourceId),
    #[error("coordination participant Target {0} is missing")]
    ParticipantMissing(EffectTargetId),
    #[error("Target {0} declares conflicting coordination contracts")]
    CoordinationContractConflict(EffectTargetId),
    #[error("Consumer returned a mismatched coordination receipt for Target {0}")]
    MismatchedCoordinationReceipt(EffectTargetId),
    #[error("Resource adapter returned a receipt for a different operation than {0}")]
    MismatchedOperationReceipt(EffectOperationId),
    #[error("operation sequence is exhausted")]
    OperationSequenceExhausted,
}
