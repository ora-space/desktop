use super::EffectWriteContext;
use super::SqliteEffectRepository;
use super::mapping::{
    effect_json, generation_to_sql, load_desired_state, resource_phase_value,
    status_version_to_sql, target_phase_value,
};
use super::source::wake_scope_targets;
use crate::DatabaseError;
use crate::TimestampSource;
use ora_effect::{
    ConditionProposal, DesiredEffect, DesiredState, EffectRepository, EffectScopeId,
    EffectTargetId, Generation, LocalTimestamp, ReplaceDesiredStateOutcome, RepositoryError,
    ResourceStatus, TargetStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

impl<Clock: TimestampSource> EffectRepository for SqliteEffectRepository<Clock> {
    fn load_desired_state(&self, scope: &EffectScopeId) -> Result<DesiredState, RepositoryError> {
        self.pool
            .with_connection(|connection| load_desired_state(connection, scope))
            .map_err(RepositoryError::new)
    }

    fn replace_desired_state(
        &self,
        scope: &EffectScopeId,
        expected_generation: Generation,
        effects: Vec<DesiredEffect>,
    ) -> Result<ReplaceDesiredStateOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let written_at = EffectWriteContext::new(&transaction, &self.clock).timestamp();
                let current = load_desired_state(&transaction, scope)?;
                if current.generation != expected_generation {
                    transaction.commit()?;
                    return Ok(ReplaceDesiredStateOutcome::Conflict {
                        expected_generation,
                        current_generation: current.generation,
                    });
                }
                let lifecycle = transaction.query_row(
                    "SELECT lifecycle FROM effect_scopes WHERE id = ?1",
                    params![scope.storage_key()],
                    |row| row.get::<_, String>(0),
                )?;
                if lifecycle == "retiring" {
                    transaction.commit()?;
                    return Ok(ReplaceDesiredStateOutcome::ScopeRetiring);
                }
                let normalized =
                    DesiredState::normalized(scope.clone(), current.generation, effects)
                        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
                for desired in normalized.effects.values() {
                    let revision = transaction
                        .query_row(
                            "SELECT availability, definition_kind
                             FROM effect_revisions WHERE id = ?1",
                            params![desired.revision.as_str()],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                        )
                        .optional()?;
                    let Some((availability, definition_kind)) = revision else {
                        transaction.commit()?;
                        return Ok(ReplaceDesiredStateOutcome::RevisionUnavailable(
                            desired.revision.clone(),
                        ));
                    };
                    if availability != "available"
                        || definition_kind != parameters_kind(&desired.parameters)
                    {
                        transaction.commit()?;
                        return Ok(ReplaceDesiredStateOutcome::RevisionUnavailable(
                            desired.revision.clone(),
                        ));
                    }
                }
                if current.effects == normalized.effects {
                    transaction.commit()?;
                    return Ok(ReplaceDesiredStateOutcome::Unchanged(current));
                }
                let generation = current
                    .generation
                    .next()
                    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
                transaction.execute(
                    "DELETE FROM effect_desired_effects WHERE scope_id = ?1",
                    params![scope.storage_key()],
                )?;
                for desired in normalized.effects.values() {
                    insert_desired_effect(&transaction, scope, desired, written_at.millis())?;
                }
                transaction.execute(
                    "UPDATE effect_scopes SET generation = ?2, updated_at = MAX(updated_at, ?3) WHERE id = ?1",
                    params![
                        scope.storage_key(),
                        generation_to_sql(generation)?,
                        written_at.millis(),
                    ],
                )?;
                wake_scope_targets(
                    &transaction,
                    &scope.storage_key(),
                    generation,
                    written_at.millis(),
                    "desired_changed",
                )?;
                transaction.execute(
                    "INSERT INTO effect_audit_events (
                         id, scope_id, subject_kind, subject_id, event_kind, generation,
                         initiator_kind, initiator_id, payload_version, payload_json, occurred_at
                     ) VALUES (?1, ?2, 'desired_state', ?2, 'desired_replaced', ?3,
                               'user', NULL, 1, '{}', ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        scope.storage_key(),
                        generation_to_sql(generation)?,
                        written_at.millis(),
                    ],
                )?;
                transaction.commit()?;
                Ok(ReplaceDesiredStateOutcome::Replaced(DesiredState {
                    scope: scope.clone(),
                    generation,
                    effects: normalized.effects,
                }))
            })
            .map_err(RepositoryError::new)
    }

    fn load_target_status(
        &self,
        target: &EffectTargetId,
    ) -> Result<Option<ora_effect::TargetStatusView>, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = connection.transaction()?;
                let view = super::read::load_target_view(&transaction, target)?;
                transaction.commit()?;
                Ok(view)
            })
            .map_err(RepositoryError::new)
    }

    fn load_consumer_target_status(
        &self,
        scope: &EffectScopeId,
        consumer: &ora_effect::ConsumerIdentity,
    ) -> Result<Option<ora_effect::TargetStatusView>, RepositoryError> {
        self.pool.with_connection_mut(|connection| {
            let transaction = connection.transaction()?;
            let target = transaction.query_row(
                "SELECT id FROM effect_targets WHERE scope_id = ?1 AND consumer_id = ?2 AND lifecycle = 'active'",
                params![scope.storage_key(), consumer.storage_key()],
                |row| row.get::<_, String>(0),
            ).optional()?.map(EffectTargetId::new);
            let view = target.map(|target| super::read::load_target_view(&transaction, &target)?.ok_or_else(|| {
                DatabaseError::CorruptEffectState("active Effect Target has no status".to_string())
            })).transpose()?;
            transaction.commit()?;
            Ok(view)
        }).map_err(RepositoryError::new)
    }

    fn request_reconcile(
        &self,
        target: &EffectTargetId,
        requested_at: LocalTimestamp,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let written_at = EffectWriteContext::new(&transaction, &self.clock).timestamp();
                let generation = transaction
                    .query_row(
                        "SELECT scope.generation
                         FROM effect_targets target
                         JOIN effect_scopes scope ON scope.id = target.scope_id
                         WHERE target.id = ?1",
                        params![target.as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                let Some(generation) = generation else {
                    transaction.commit()?;
                    return Ok(false);
                };
                super::declaration::upsert_target_wakeup(
                    &transaction,
                    target.as_str(),
                    super::mapping::generation_from_sql(generation)?,
                    requested_at.millis(),
                    "user_requested",
                    written_at.millis(),
                )?;
                transaction.commit()?;
                Ok(true)
            })
            .map_err(RepositoryError::new)
    }

    // Durable claim methods stay in `queue`; journal transitions use the same centralized fences.
    fn claim_due_targets(
        &self,
        worker: &ora_effect::WorkerIdentity,
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
        limit: usize,
    ) -> Result<Vec<(EffectTargetId, ora_effect::ReconcileClaim)>, RepositoryError> {
        super::queue::claim_due_targets(self, worker, now, lease_until, limit)
            .map_err(RepositoryError::new)
    }

    fn load_reconcile_snapshot(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
    ) -> Result<ora_effect::ReconcileSnapshot, RepositoryError> {
        super::queue::load_reconcile_snapshot(self, target, claim).map_err(RepositoryError::new)
    }

    fn claim_resources(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        resources: &[ora_effect::EffectResourceId],
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
    ) -> Result<Option<Vec<ora_effect::ResourceClaim>>, RepositoryError> {
        super::queue::claim_resources(self, target, claim, resources, now, lease_until)
            .map_err(RepositoryError::new)
    }

    fn prepare_attempt(
        &self,
        claim: &ora_effect::ReconcileClaim,
        attempt: ora_effect::ReconcileAttempt,
        target_projections: Vec<ora_effect::TargetProjection>,
        resource_projections: Vec<ora_effect::ResourceProjection>,
        operations: Vec<ora_effect::EffectOperation>,
        artifacts: Vec<ora_effect::OperationArtifact>,
    ) -> Result<(), RepositoryError> {
        super::journal::prepare_attempt(
            self,
            claim,
            attempt,
            target_projections,
            resource_projections,
            operations,
            artifacts,
        )
        .map_err(RepositoryError::new)
    }

    fn record_attempt_progress(
        &self,
        claim: &ora_effect::ReconcileClaim,
        attempt: &ora_effect::ReconcileAttempt,
        operations: &[ora_effect::EffectOperation],
        coordination_receipts: &[ora_effect::CoordinationReceipt],
    ) -> Result<(), RepositoryError> {
        super::journal::record_attempt_progress(
            self,
            claim,
            attempt,
            operations,
            coordination_receipts,
        )
        .map_err(RepositoryError::new)
    }

    fn block_target(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        target_status: TargetStatus,
        resource_statuses: Vec<ResourceStatus>,
        conditions: Vec<ConditionProposal>,
    ) -> Result<(), RepositoryError> {
        super::queue::block_target(
            self,
            target,
            claim,
            target_status,
            resource_statuses,
            conditions,
        )
        .map_err(RepositoryError::new)
    }

    fn commit_projection(
        &self,
        claim: &ora_effect::ReconcileClaim,
        commit: ora_effect::ProjectionCommit,
    ) -> Result<(), RepositoryError> {
        super::queue::commit_projection(self, claim, commit).map_err(RepositoryError::new)
    }

    fn finalize_attempt(
        &self,
        claim: &ora_effect::ReconcileClaim,
        finalization: ora_effect::AttemptFinalization,
    ) -> Result<(), RepositoryError> {
        super::journal::finalize_attempt(self, claim, finalization).map_err(RepositoryError::new)
    }

    fn schedule_retry(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        not_before: LocalTimestamp,
        scheduled_at: LocalTimestamp,
    ) -> Result<Option<ora_effect::RetryAttempt>, RepositoryError> {
        super::queue::schedule_retry(self, target, claim, not_before, scheduled_at)
            .map_err(RepositoryError::new)
    }

    fn load_unfinished_operations(
        &self,
    ) -> Result<Vec<ora_effect::EffectOperation>, RepositoryError> {
        super::recovery::load_unfinished_operations(self).map_err(RepositoryError::new)
    }

    fn quarantine_unfinished_operations(
        &self,
        detected_at: LocalTimestamp,
    ) -> Result<usize, RepositoryError> {
        super::recovery::quarantine_unfinished_operations(self, detected_at)
            .map_err(RepositoryError::new)
    }

    fn complete_artifact_cleanup(
        &self,
        artifact: &ora_effect::ArtifactId,
        receipt: ora_effect::CleanupReceipt,
    ) -> Result<(), RepositoryError> {
        super::recovery::complete_artifact_cleanup(self, artifact, receipt)
            .map_err(RepositoryError::new)
    }

    fn mark_artifact_cleanup_failed(
        &self,
        artifact: ora_effect::OperationArtifact,
    ) -> Result<(), RepositoryError> {
        super::recovery::mark_artifact_cleanup_failed(self, artifact).map_err(RepositoryError::new)
    }
}

/// Inserts one fully typed Desired Effect row at a well-defined persistence boundary.
fn insert_desired_effect(
    transaction: &Transaction<'_>,
    scope: &EffectScopeId,
    desired: &DesiredEffect,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_desired_effects (
             id, scope_id, revision_id, parameters_kind, parameters_version,
             parameters_json, selector_version, selector_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 1, ?5, 1, ?6, ?7, ?7)",
        params![
            desired.identity.as_str(),
            scope.storage_key(),
            desired.revision.as_str(),
            parameters_kind(&desired.parameters),
            effect_json(&desired.parameters)?,
            effect_json(&desired.audience)?,
            updated_at,
        ],
    )?;
    Ok(())
}

/// Returns the persisted kind discriminator for typed parameters.
fn parameters_kind(parameters: &ora_effect::ValidatedEffectParameters) -> &'static str {
    match parameters {
        ora_effect::ValidatedEffectParameters::Skill(_) => "skill",
    }
}

/// Persists one Target status only through a validated domain snapshot.
pub(super) fn save_target_status(
    transaction: &Transaction<'_>,
    status: &TargetStatus,
    written_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let progress = status.progress();
    let (phase, recovery) = target_phase_value(status.phase());
    transaction.execute(
        "UPDATE effect_target_status SET
             desired_generation = ?2, observed_generation = ?3, applied_generation = ?4,
             ready_generation = ?5, phase = ?6, recovery_operation_id = ?7,
             status_version = ?8, updated_at = MAX(updated_at, ?9)
         WHERE target_id = ?1",
        params![
            status.target().as_str(),
            generation_to_sql(progress.desired())?,
            generation_to_sql(progress.observed())?,
            generation_to_sql(progress.applied())?,
            generation_to_sql(progress.ready())?,
            phase,
            recovery,
            status_version_to_sql(status.version())?,
            written_at.millis(),
        ],
    )?;
    Ok(())
}

/// Persists one Resource status only through a validated domain snapshot.
pub(super) fn save_resource_status(
    transaction: &Transaction<'_>,
    status: &ResourceStatus,
    written_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let (phase, recovery) = resource_phase_value(status.phase());
    transaction.execute(
        "UPDATE effect_resource_status SET
             desired_generation = ?2, observed_generation = ?3, applied_generation = ?4,
             phase = ?5, recovery_operation_id = ?6, status_version = ?7, updated_at = MAX(updated_at, ?8)
         WHERE resource_id = ?1",
        params![
            status.resource().as_str(),
            generation_to_sql(status.desired())?,
            generation_to_sql(status.observed())?,
            generation_to_sql(status.applied())?,
            phase,
            recovery,
            status_version_to_sql(status.version())?,
            written_at.millis(),
        ],
    )?;
    Ok(())
}
