use crate::{
    Condition, ConditionReason, ConditionSubject, ConsumerCoordinator, ConsumerStatus,
    CoordinationOutcome, DesiredSkillState, EffectOperation, EffectOperationId,
    EffectOperationKind, EffectOperationPhase, EffectRepository, FilesystemEffectError,
    FilesystemSurfaceAdapter, Generation, LedgerTransition, ManagedIdentity,
    ManagedIdentityGenerator, ManagedSkill, OperationPaths, OperationState, PlanOperation,
    PlanOperationKind, Planner, PlannerInput, RecoveryDecision, RepositoryError, SourceError,
    SourceProvider, SurfaceDescriptorSet, SurfaceLifecycle, SurfacePhase, SurfaceStatus,
};
use std::collections::BTreeMap;
use thiserror::Error;

/// Result of one explicitly driven surface reconcile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileOutcome {
    pub status: SurfaceStatus,
    pub consumer_statuses: Vec<ConsumerStatus>,
}

/// Reports failures that prevent a complete scan or durable status transition.
#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Filesystem(#[from] FilesystemEffectError),
}

/// Statically dispatched orchestrator for one-process Effect reconciliation.
pub struct Reconciler<'a, Repository, Sources, Coordinator, IdentityGenerator> {
    repository: &'a Repository,
    sources: &'a Sources,
    coordinator: &'a Coordinator,
    identity_generator: &'a IdentityGenerator,
}

/// Groups the immutable inputs needed to create one durable filesystem operation.
struct OperationRequest<'a> {
    adapter: &'a FilesystemSurfaceAdapter,
    workspace_id: &'a ora_domain::WorkspaceId,
    surface_key: &'a crate::SurfaceKey,
    generation: Generation,
    plan: &'a PlanOperation,
    operation_id: EffectOperationId,
    paths: OperationPaths,
}

/// Selects whether this status write preserves or advances the surface file generation.
enum AppliedGenerationUpdate {
    Preserve,
    Advance(Generation),
}

/// Groups one complete status revision update.
struct StatusUpdate<'a> {
    descriptor: &'a SurfaceDescriptorSet,
    workspace_id: &'a ora_domain::WorkspaceId,
    desired_generation: Generation,
    phase: SurfacePhase,
    conditions: Vec<Condition>,
    occurred_at: i64,
    applied: AppliedGenerationUpdate,
}

impl<'a, Repository, Sources, Coordinator, IdentityGenerator>
    Reconciler<'a, Repository, Sources, Coordinator, IdentityGenerator>
where
    Repository: EffectRepository,
    Sources: SourceProvider,
    Coordinator: ConsumerCoordinator,
    IdentityGenerator: ManagedIdentityGenerator,
{
    pub fn new(
        repository: &'a Repository,
        sources: &'a Sources,
        coordinator: &'a Coordinator,
        identity_generator: &'a IdentityGenerator,
    ) -> Self {
        Self {
            repository,
            sources,
            coordinator,
            identity_generator,
        }
    }

    /// Recovers older durable transactions, computes a fresh plan, and applies every safe locator.
    pub fn reconcile_surface(
        &self,
        adapter: &FilesystemSurfaceAdapter,
        descriptor: &SurfaceDescriptorSet,
        workspace_id: &ora_domain::WorkspaceId,
        occurred_at: i64,
    ) -> Result<ReconcileOutcome, ReconcileError> {
        let effect = self.repository.load_workspace_effect(workspace_id)?;
        let mut conditions = self.recover_surface_operations(
            adapter,
            workspace_id,
            &descriptor.surface_key,
            effect.generation,
            occurred_at,
        )?;
        if conditions
            .iter()
            .any(|condition| condition.reason == ConditionReason::RecoveryRequired)
        {
            let status = self.persist_status(StatusUpdate {
                descriptor,
                workspace_id,
                desired_generation: effect.generation,
                phase: SurfacePhase::RecoveryRequired,
                conditions,
                occurred_at,
                applied: AppliedGenerationUpdate::Preserve,
            })?;
            return Ok(ReconcileOutcome {
                status,
                consumer_statuses: Vec::new(),
            });
        }

        let managed = self
            .repository
            .load_managed_skills(workspace_id, &descriptor.surface_key)?;
        let scan = adapter.scan()?;
        let planner = Planner::new(self.identity_generator);
        let plan = planner.plan(PlannerInput {
            surface_key: &descriptor.surface_key,
            lifecycle: descriptor.lifecycle,
            generation: effect.generation,
            desired: &effect.spec.skills,
            managed: &managed,
            observed: &scan.targets,
            occurred_at,
        });
        conditions.extend(plan.conditions.clone());

        if descriptor.lifecycle == SurfaceLifecycle::Active
            && descriptor.consumers.is_empty()
            && !effect.spec.skills.is_empty()
        {
            conditions.push(Condition::new(
                ConditionSubject::Surface {
                    surface_key: descriptor.surface_key.clone(),
                },
                ConditionReason::NoConsumers,
                "desired Skills have no active surface consumer",
                occurred_at,
                effect.generation,
            ));
        }

        let coordinated_consumers = descriptor.consumers.keys().cloned().collect::<Vec<_>>();
        let mut quiesced = false;
        if plan.has_filesystem_mutations() && descriptor.requires_coordination() {
            match self
                .coordinator
                .quiesce(&descriptor.surface_key, &coordinated_consumers)
            {
                Ok(CoordinationOutcome::Ready) => quiesced = true,
                Ok(CoordinationOutcome::WaitingForIdle) | Err(_) => {
                    conditions.push(Condition::new(
                        ConditionSubject::Surface {
                            surface_key: descriptor.surface_key.clone(),
                        },
                        ConditionReason::WaitingForIdle,
                        "a surface consumer has not reached an idle mutation boundary",
                        occurred_at,
                        effect.generation,
                    ));
                    let status = self.persist_status(StatusUpdate {
                        descriptor,
                        workspace_id,
                        desired_generation: effect.generation,
                        phase: SurfacePhase::WaitingForIdle,
                        conditions,
                        occurred_at,
                        applied: AppliedGenerationUpdate::Preserve,
                    })?;
                    return Ok(ReconcileOutcome {
                        status,
                        consumer_statuses: Vec::new(),
                    });
                }
            }
        }

        for operation in &plan.operations {
            if let Err(condition) = self.execute_operation(
                adapter,
                workspace_id,
                &descriptor.surface_key,
                effect.generation,
                operation,
                occurred_at,
            ) {
                conditions.push(*condition);
            }
        }

        // A second live scan prevents a partial-success generation from being marked Current.
        let final_managed = self
            .repository
            .load_managed_skills(workspace_id, &descriptor.surface_key)?;
        let final_scan = adapter.scan()?;
        let final_plan = planner.plan(PlannerInput {
            surface_key: &descriptor.surface_key,
            lifecycle: descriptor.lifecycle,
            generation: effect.generation,
            desired: &effect.spec.skills,
            managed: &final_managed,
            observed: &final_scan.targets,
            occurred_at,
        });
        let files_current = final_plan.is_current();
        merge_conditions(&mut conditions, final_plan.conditions);
        let phase = if files_current && conditions.is_empty() {
            SurfacePhase::Current
        } else if descriptor.lifecycle == SurfaceLifecycle::Retiring {
            SurfacePhase::Retiring
        } else {
            SurfacePhase::Degraded
        };
        let applied = if files_current {
            AppliedGenerationUpdate::Advance(effect.generation)
        } else {
            AppliedGenerationUpdate::Preserve
        };
        let status = self.persist_status(StatusUpdate {
            descriptor,
            workspace_id,
            desired_generation: effect.generation,
            phase,
            conditions,
            occurred_at,
            applied,
        })?;

        let mut consumer_statuses = Vec::new();
        if quiesced || (files_current && !coordinated_consumers.is_empty()) {
            for consumer in coordinated_consumers {
                let (consumer_phase, ready_generation, consumer_conditions) = match self
                    .coordinator
                    .resume(&descriptor.surface_key, &consumer, effect.generation)
                {
                    Ok(()) if files_current => {
                        (SurfacePhase::Current, effect.generation, Vec::new())
                    }
                    Ok(()) => (SurfacePhase::Degraded, Generation::default(), Vec::new()),
                    Err(_) => (
                        SurfacePhase::Degraded,
                        Generation::default(),
                        vec![Condition::new(
                            ConditionSubject::Consumer {
                                consumer_id: consumer.clone(),
                            },
                            ConditionReason::ConsumerResumeFailed,
                            "surface consumer failed to resume",
                            occurred_at,
                            effect.generation,
                        )],
                    ),
                };
                let consumer_status = ConsumerStatus {
                    surface_key: descriptor.surface_key.clone(),
                    consumer_id: consumer,
                    ready_generation,
                    phase: consumer_phase,
                    revision: 1,
                    updated_at: occurred_at,
                    conditions: consumer_conditions,
                };
                self.repository
                    .save_consumer_status(consumer_status.clone())?;
                consumer_statuses.push(consumer_status);
            }
        }

        Ok(ReconcileOutcome {
            status,
            consumer_statuses,
        })
    }

    /// Applies one safe planned locator through Prepared, Applied, and atomic ledger Finalized.
    fn execute_operation(
        &self,
        adapter: &FilesystemSurfaceAdapter,
        workspace_id: &ora_domain::WorkspaceId,
        surface_key: &crate::SurfaceKey,
        generation: Generation,
        plan: &PlanOperation,
        occurred_at: i64,
    ) -> Result<(), Box<Condition>> {
        if let PlanOperationKind::AdvanceGeneration { previous } = &plan.kind {
            let mut advanced = previous.clone();
            advanced.applied_generation = generation;
            return self.repository.save_managed_skill(advanced).map_err(|_| {
                Box::new(Condition::new(
                    ConditionSubject::ManagedSkill {
                        managed_identity: previous.managed_identity.clone(),
                    },
                    ConditionReason::TransientIo,
                    "the managed generation could not be persisted",
                    occurred_at,
                    generation,
                ))
            });
        }
        let operation_id = EffectOperationId::random();
        let paths = OperationPaths::for_operation(&adapter.surface_root(), &operation_id);
        let result = self.prepare_and_apply(OperationRequest {
            adapter,
            workspace_id,
            surface_key,
            generation,
            plan,
            operation_id,
            paths,
        });
        result.map_err(|reason| {
            let (condition_reason, message) = match reason {
                OperationFailure::Source(_) => (
                    ConditionReason::SourceUnavailable,
                    "the selected source revision is unavailable",
                ),
                OperationFailure::Filesystem(_) => (
                    ConditionReason::MaterializationFailed,
                    "the safe filesystem operation failed",
                ),
                OperationFailure::Repository(_) => (
                    ConditionReason::TransientIo,
                    "the durable Effect operation could not be persisted",
                ),
            };
            Box::new(Condition::new(
                operation_subject(plan, surface_key),
                condition_reason,
                message,
                occurred_at,
                generation,
            ))
        })
    }

    /// Builds durable intent from the live previous disk state and then performs its exact swap.
    fn prepare_and_apply(&self, request: OperationRequest<'_>) -> Result<(), OperationFailure> {
        let OperationRequest {
            adapter,
            workspace_id,
            surface_key,
            generation,
            plan,
            operation_id,
            paths,
        } = request;
        let (
            kind,
            target_name,
            previous_managed,
            planned_desired,
            previous_identity,
            planned_identity,
        ) = operation_parts(plan);
        let previous_state = adapter.operation_state(&target_name)?;
        let staged_source = if let Some(desired) = &planned_desired {
            let snapshot = self.sources.open_snapshot(desired)?;
            let identity = planned_identity
                .as_ref()
                .or(previous_identity.as_ref())
                .ok_or(SourceError::IntegrityMismatch)?;
            Some((snapshot, identity.clone()))
        } else {
            None
        };
        let planned_state = if let Some((snapshot, _)) = &staged_source {
            OperationState::Present(adapter.planned_fingerprint(snapshot)?)
        } else {
            OperationState::Missing
        };
        let mut operation = EffectOperation {
            operation_id,
            generation,
            workspace_id: workspace_id.clone(),
            surface_key: surface_key.clone(),
            locator: plan.locator.clone(),
            target_name,
            kind,
            phase: EffectOperationPhase::Prepared,
            previous_state,
            planned_state,
            previous_identity,
            planned_identity,
            previous_managed,
            planned_desired,
            staging_path: paths.staging.clone(),
            backup_path: paths.backup.clone(),
        };
        self.repository.prepare_operation(operation.clone())?;
        if let Some((snapshot, identity)) = &staged_source {
            let staged_fingerprint = adapter.stage(snapshot, identity, &paths)?;
            if operation.planned_state != OperationState::Present(staged_fingerprint) {
                return Err(SourceError::IntegrityMismatch.into());
            }
        }
        apply_prepared(adapter, &operation, &paths)?;
        operation.phase = EffectOperationPhase::Applied;
        self.repository.save_operation(operation.clone())?;
        let transition = ledger_transition(&operation)?;
        operation.phase = EffectOperationPhase::Finalized;
        self.repository
            .finalize_operation(operation.clone(), transition)?;
        // Business state is already committed; cleanup remains safely retryable maintenance.
        if adapter.cleanup_operation(&paths).is_ok() {
            let _ = self
                .repository
                .complete_operation_cleanup(&operation.operation_id);
        }
        Ok(())
    }

    /// Resolves unfinished operations before allowing a newer generation to plan more mutations.
    fn recover_surface_operations(
        &self,
        adapter: &FilesystemSurfaceAdapter,
        workspace_id: &ora_domain::WorkspaceId,
        surface_key: &crate::SurfaceKey,
        generation: Generation,
        occurred_at: i64,
    ) -> Result<Vec<Condition>, ReconcileError> {
        let mut conditions = Vec::new();
        for mut operation in self.repository.load_unfinished_operations()? {
            if operation.workspace_id != *workspace_id || operation.surface_key != *surface_key {
                continue;
            }
            let paths = OperationPaths {
                root: operation
                    .staging_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| adapter.surface_root()),
                staging: operation.staging_path.clone(),
                backup: operation.backup_path.clone(),
            };
            match adapter.recovery_decision(&operation)? {
                RecoveryDecision::RetryApply => {
                    stage_recovery_source(adapter, self.sources, &operation, &paths)?;
                    apply_prepared(adapter, &operation, &paths)?;
                    operation.phase = EffectOperationPhase::Applied;
                    self.repository.save_operation(operation.clone())?;
                    let transition = ledger_transition(&operation)
                        .map_err(|_| FilesystemEffectError::InvalidSkillManifest)?;
                    operation.phase = EffectOperationPhase::Finalized;
                    self.repository
                        .finalize_operation(operation.clone(), transition)?;
                    if adapter.cleanup_operation(&paths).is_ok() {
                        let _ = self
                            .repository
                            .complete_operation_cleanup(&operation.operation_id);
                    }
                }
                RecoveryDecision::Finalize => {
                    let transition = ledger_transition(&operation)
                        .map_err(|_| FilesystemEffectError::InvalidSkillManifest)?;
                    operation.phase = EffectOperationPhase::Finalized;
                    self.repository
                        .finalize_operation(operation.clone(), transition)?;
                    if adapter.cleanup_operation(&paths).is_ok() {
                        let _ = self
                            .repository
                            .complete_operation_cleanup(&operation.operation_id);
                    }
                }
                RecoveryDecision::RecoveryRequired => conditions.push(Condition::new(
                    ConditionSubject::Surface {
                        surface_key: surface_key.clone(),
                    },
                    ConditionReason::RecoveryRequired,
                    "the target matches neither the previous nor planned operation state",
                    occurred_at,
                    generation,
                )),
            }
        }
        Ok(conditions)
    }

    /// Writes status with its own revision and preserves the previous applied generation on errors.
    fn persist_status(&self, update: StatusUpdate<'_>) -> Result<SurfaceStatus, RepositoryError> {
        let StatusUpdate {
            descriptor,
            workspace_id,
            desired_generation,
            phase,
            conditions,
            occurred_at,
            applied,
        } = update;
        let previous = self
            .repository
            .load_surface_status(workspace_id, &descriptor.surface_key)?;
        let status = SurfaceStatus {
            workspace_id: workspace_id.clone(),
            surface_key: descriptor.surface_key.clone(),
            desired_generation,
            observed_generation: desired_generation,
            applied_generation: match applied {
                AppliedGenerationUpdate::Advance(generation) => generation,
                AppliedGenerationUpdate::Preserve => previous
                    .as_ref()
                    .map_or_else(Generation::default, |status| status.applied_generation),
            },
            phase,
            revision: previous.map_or(1, |status| status.revision.saturating_add(1)),
            updated_at: occurred_at,
            conditions,
        };
        self.repository.save_surface_status(status.clone())?;
        Ok(status)
    }
}

#[derive(Debug, Error)]
enum OperationFailure {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Filesystem(#[from] FilesystemEffectError),
}

/// Extracts operation-specific state while keeping illegal combinations in the plan enum.
fn operation_parts(
    plan: &PlanOperation,
) -> (
    EffectOperationKind,
    crate::SkillName,
    Option<ManagedSkill>,
    Option<DesiredSkillState>,
    Option<ManagedIdentity>,
    Option<ManagedIdentity>,
) {
    match &plan.kind {
        PlanOperationKind::Create {
            desired,
            managed_identity,
        } => (
            EffectOperationKind::Create,
            desired.state().name.clone(),
            None,
            Some(desired.clone()),
            None,
            Some(managed_identity.clone()),
        ),
        PlanOperationKind::Update { previous, desired } => (
            EffectOperationKind::Update,
            desired.state().name.clone(),
            Some(previous.clone()),
            Some(desired.clone()),
            Some(previous.managed_identity.clone()),
            Some(previous.managed_identity.clone()),
        ),
        PlanOperationKind::AdvanceGeneration { previous } => (
            EffectOperationKind::Update,
            previous.target_name.clone(),
            Some(previous.clone()),
            Some(previous.state.clone()),
            Some(previous.managed_identity.clone()),
            Some(previous.managed_identity.clone()),
        ),
        PlanOperationKind::Replace {
            previous,
            desired,
            managed_identity,
        } => (
            EffectOperationKind::Replace,
            desired.state().name.clone(),
            Some(previous.clone()),
            Some(desired.clone()),
            Some(previous.managed_identity.clone()),
            Some(managed_identity.clone()),
        ),
        PlanOperationKind::Delete { previous } => (
            EffectOperationKind::Delete,
            previous.target_name.clone(),
            Some(previous.clone()),
            None,
            Some(previous.managed_identity.clone()),
            None,
        ),
    }
}

/// Rebuilds or validates a reserved staging artifact before replaying Prepared intent.
fn stage_recovery_source<Sources: SourceProvider>(
    adapter: &FilesystemSurfaceAdapter,
    sources: &Sources,
    operation: &EffectOperation,
    paths: &OperationPaths,
) -> Result<(), FilesystemEffectError> {
    let Some(desired) = &operation.planned_desired else {
        return Ok(());
    };
    let snapshot = sources
        .open_snapshot(desired)
        .map_err(|_| FilesystemEffectError::InvalidSkillManifest)?;
    let identity = operation
        .planned_identity
        .as_ref()
        .or(operation.previous_identity.as_ref())
        .ok_or(FilesystemEffectError::InvalidSkillManifest)?;
    let staged_fingerprint = adapter.stage(&snapshot, identity, paths)?;
    if operation.planned_state != OperationState::Present(staged_fingerprint) {
        return Err(FilesystemEffectError::InvalidSkillManifest);
    }
    Ok(())
}

/// Applies only the exact mutation described by a durable Prepared operation.
fn apply_prepared(
    adapter: &FilesystemSurfaceAdapter,
    operation: &EffectOperation,
    paths: &OperationPaths,
) -> Result<(), FilesystemEffectError> {
    match operation.kind {
        EffectOperationKind::Create => adapter.apply_create(&operation.target_name, paths),
        EffectOperationKind::Update | EffectOperationKind::Replace => {
            let previous_name = operation
                .previous_managed
                .as_ref()
                .map_or(&operation.target_name, |managed| &managed.target_name);
            adapter.apply_swap(previous_name, &operation.target_name, paths)
        }
        EffectOperationKind::Delete => adapter.apply_delete(&operation.target_name, paths),
    }
}

/// Builds the ledger half of finalization from the operation's represented state machine.
fn ledger_transition(operation: &EffectOperation) -> Result<LedgerTransition, SourceError> {
    match operation.kind {
        EffectOperationKind::Create | EffectOperationKind::Update => {
            Ok(LedgerTransition::Upsert(planned_managed(operation)?))
        }
        EffectOperationKind::Replace => Ok(LedgerTransition::Replace {
            previous_identity: operation
                .previous_identity
                .clone()
                .ok_or(SourceError::IntegrityMismatch)?,
            next: planned_managed(operation)?,
        }),
        EffectOperationKind::Delete => Ok(LedgerTransition::Delete {
            managed_identity: operation
                .previous_identity
                .clone()
                .ok_or(SourceError::IntegrityMismatch)?,
        }),
    }
}

/// Constructs the exact managed ledger only after the planned fingerprint is visible.
fn planned_managed(operation: &EffectOperation) -> Result<ManagedSkill, SourceError> {
    let desired = operation
        .planned_desired
        .clone()
        .ok_or(SourceError::IntegrityMismatch)?;
    let fingerprint = match &operation.planned_state {
        OperationState::Present(fingerprint) => fingerprint.clone(),
        OperationState::Missing => return Err(SourceError::IntegrityMismatch),
    };
    let selection_key = desired
        .state()
        .source
        .selection_key(desired.state().name.clone())
        .ok_or(SourceError::IntegrityMismatch)?;
    Ok(ManagedSkill {
        managed_identity: operation
            .planned_identity
            .clone()
            .ok_or(SourceError::IntegrityMismatch)?,
        workspace_id: operation.workspace_id.clone(),
        surface_key: operation.surface_key.clone(),
        selection_key,
        locator: operation.locator.clone(),
        target_name: operation.target_name.clone(),
        state: desired,
        applied_fingerprint: fingerprint,
        applied_generation: operation.generation,
    })
}

/// Assigns per-resource failures to the most specific stable condition subject available.
fn operation_subject(plan: &PlanOperation, _surface_key: &crate::SurfaceKey) -> ConditionSubject {
    match &plan.kind {
        PlanOperationKind::Create { desired, .. } => ConditionSubject::DesiredSkill {
            selection_key: desired
                .state()
                .source
                .selection_key(desired.state().name.clone())
                .unwrap_or_else(|| {
                    crate::SkillSelectionKey::new(
                        crate::SourceKind::Local,
                        ora_domain::Namespace::local(),
                        desired.state().name.clone(),
                    )
                }),
        },
        PlanOperationKind::Update { previous, .. }
        | PlanOperationKind::AdvanceGeneration { previous }
        | PlanOperationKind::Replace { previous, .. }
        | PlanOperationKind::Delete { previous } => ConditionSubject::ManagedSkill {
            managed_identity: previous.managed_identity.clone(),
        },
    }
}

/// Deduplicates current conditions by stable subject and reason while retaining safe messages.
fn merge_conditions(current: &mut Vec<Condition>, additional: Vec<Condition>) {
    let mut keys = BTreeMap::new();
    for condition in current.iter().chain(additional.iter()) {
        keys.insert(
            format!("{:?}:{:?}", condition.subject, condition.reason),
            condition.clone(),
        );
    }
    *current = keys.into_values().collect();
}
