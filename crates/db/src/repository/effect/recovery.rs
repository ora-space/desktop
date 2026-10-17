use super::EffectWriteContext;
use super::SqliteEffectRepository;
use super::conditions::replace_conditions;
use super::mapping::{
    effect_json, generation_from_sql, generation_to_sql, load_resource_status, load_target,
    load_target_status,
};
use super::operations::{map_operation, update_operation};
use super::persistence::parse_attempt_phase;
use super::source::advance_topology_epochs;
use super::store::{save_resource_status, save_target_status};
use crate::DatabaseError;
use crate::TimestampSource;
use ora_effect::{
    CleanupReceipt, ConditionOwner, Digest, EffectOperation, EffectTargetId, LocalTimestamp,
    OperationArtifact, ProjectionDigest, RecoveryAttemptJournal, ReconcileAttempt,
    ReconcileAttemptId, ReconcileAttemptIntent, TargetRecoveryFinalization, TargetRecoveryJournal,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Loads unfinished immutable operation journals in deterministic preparation order.
pub(super) fn load_unfinished_operations<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
) -> Result<Vec<EffectOperation>, DatabaseError> {
    repository.pool.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT id, attempt_id, resource_id, generation, sequence, mutation,
                    expected_json, planned_json, payload_json, phase,
                    prepared_at, applied_at, finalized_at, updated_at, detected_at
             FROM effect_operations WHERE phase NOT IN ('finalized', 'finalized_without_effect')
             ORDER BY prepared_at, attempt_id, sequence, id",
        )?;
        let mut rows = statement.query([])?;
        let mut operations = Vec::new();
        while let Some(row) = rows.next()? {
            operations.push(map_operation(row)?);
        }
        Ok(operations)
    })
}

/// Loads one Target with every quarantined journal that blocks its explicit recovery.
pub(super) fn load_target_recovery<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    target_id: &EffectTargetId,
) -> Result<Option<TargetRecoveryJournal>, DatabaseError> {
    repository.pool.with_connection(|connection| {
        let exists = connection
            .query_row(
                "SELECT id FROM effect_targets WHERE id = ?1",
                params![target_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(None);
        }
        let target = load_target(connection, target_id)?;
        let target_status = load_target_status(connection, target_id)?
            .ok_or_else(|| {
                DatabaseError::CorruptEffectState(format!(
                    "Effect Target {target_id} has no status"
                ))
            })?;
        let attempts = load_recovery_attempts(connection, target_id)?;
        Ok(Some(TargetRecoveryJournal {
            target,
            target_status,
            attempts,
        }))
    })
}

/// Restores every quarantined attempt with its operation journals and retained artifacts.
fn load_recovery_attempts(
    connection: &rusqlite::Connection,
    target_id: &EffectTargetId,
) -> Result<Vec<RecoveryAttemptJournal>, DatabaseError> {
    let mut attempt_statement = connection.prepare(
        "SELECT id, generation, consumer_revision_id, target_projection_digest,
                coordination_plan_version, coordination_plan_json, phase
         FROM effect_reconcile_attempts
         WHERE target_id = ?1 AND phase = 'recovery_required'
         ORDER BY prepared_at, id",
    )?;
    let attempt_rows = attempt_statement
        .query_map(params![target_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(attempt_statement);

    let mut journals = Vec::new();
    for (id, generation, consumer_revision, projection, coordination_json, phase) in attempt_rows {
        let attempt_id = ReconcileAttemptId::new(id);
        let mut operation_statement = connection.prepare(
            "SELECT id, attempt_id, resource_id, generation, sequence, mutation,
                    expected_json, planned_json, payload_json, phase,
                    prepared_at, applied_at, finalized_at, updated_at, detected_at
             FROM effect_operations
             WHERE attempt_id = ?1 AND phase = 'recovery_required'
             ORDER BY sequence, id",
        )?;
        let mut rows = operation_statement.query(params![attempt_id.as_str()])?;
        let mut operations = Vec::new();
        while let Some(row) = rows.next()? {
            operations.push(map_operation(row)?);
        }
        drop(rows);
        drop(operation_statement);
        let operation_ids = operations
            .iter()
            .map(|operation| operation.identity().clone())
            .collect::<Vec<_>>();
        let mut projection_statement = connection.prepare(
            "SELECT resource_projection_digest FROM effect_attempt_resource_projections
             WHERE attempt_id = ?1 ORDER BY sequence",
        )?;
        let mut resource_projections = BTreeSet::new();
        let mut projection_rows =
            projection_statement.query(params![attempt_id.as_str()])?;
        while let Some(row) = projection_rows.next()? {
            let digest: String = row.get(0)?;
            resource_projections.insert(ProjectionDigest::new(
                Digest::parse(&digest)
                    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
            ));
        }
        drop(projection_rows);
        drop(projection_statement);
        let attempt = ReconcileAttempt::restore(
            attempt_id.clone(),
            ReconcileAttemptIntent {
                target: target_id.clone(),
                generation: generation_from_sql(generation)?,
                consumer_revision: ora_effect::ConsumerRevisionId::new(consumer_revision),
                target_projection: ProjectionDigest::new(
                    Digest::parse(&projection)
                        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
                ),
                resource_projections,
                coordination: serde_json::from_str(&coordination_json)
                    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
                operations: operation_ids,
            },
            parse_attempt_phase(&phase)?,
        )
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        journals.push(RecoveryAttemptJournal {
            attempt,
            operations,
            artifacts: load_attempt_artifacts(connection, &attempt_id)?,
        });
    }
    Ok(journals)
}

/// Loads every retained cleanup artifact owned by one attempt's operation journals.
fn load_attempt_artifacts(
    connection: &rusqlite::Connection,
    attempt: &ReconcileAttemptId,
) -> Result<Vec<OperationArtifact>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT artifact.id, artifact.operation_id, artifact.role,
                artifact.locator_json, artifact.expected_fingerprint, artifact.state
         FROM effect_operation_artifacts artifact
         JOIN effect_operations operation ON operation.id = artifact.operation_id
         WHERE operation.attempt_id = ?1
         ORDER BY artifact.id",
    )?;
    let artifacts = statement
        .query_map(params![attempt.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut restored = Vec::new();
    for (identity, operation, role, locator_json, fingerprint, state) in artifacts {
        restored.push(OperationArtifact {
            identity: ora_effect::ArtifactId::new(identity),
            operation: ora_effect::EffectOperationId::new(operation),
            role: parse_artifact_role(&role)?,
            locator: serde_json::from_str(&locator_json)
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
            expected_fingerprint: ora_effect::Fingerprint::parse(&fingerprint)
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
            state: parse_artifact_state(&state)?,
        });
    }
    Ok(restored)
}

/// Decodes the closed artifact role set from storage.
fn parse_artifact_role(value: &str) -> Result<ora_effect::ArtifactRole, DatabaseError> {
    match value {
        "staging" => Ok(ora_effect::ArtifactRole::Staging),
        "backup" => Ok(ora_effect::ArtifactRole::Backup),
        "compensation" => Ok(ora_effect::ArtifactRole::Compensation),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect artifact role {other}"
        ))),
    }
}

/// Decodes the closed artifact cleanup state from storage.
fn parse_artifact_state(value: &str) -> Result<ora_effect::ArtifactState, DatabaseError> {
    match value {
        "reserved" => Ok(ora_effect::ArtifactState::Reserved),
        "retained" => Ok(ora_effect::ArtifactState::Retained),
        "pending_cleanup" => Ok(ora_effect::ArtifactState::PendingCleanup),
        "cleanup_failed" => Ok(ora_effect::ArtifactState::CleanupFailed),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect artifact state {other}"
        ))),
    }
}

/// Quarantines ambiguous unfinished journals and blocks their Targets for explicit recovery.
pub(super) fn quarantine_unfinished_operations<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    detected_at: LocalTimestamp,
) -> Result<usize, DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        let operations = {
            let mut statement = transaction.prepare(
                "SELECT operation.id, operation.attempt_id, operation.resource_id,
                        operation.generation, operation.sequence, operation.mutation,
                        operation.expected_json, operation.planned_json, operation.payload_json,
                        operation.phase, operation.prepared_at, operation.applied_at,
                        operation.finalized_at, operation.updated_at, operation.detected_at, attempt.target_id
                 FROM effect_operations operation
                 JOIN effect_reconcile_attempts attempt ON attempt.id = operation.attempt_id
                 WHERE operation.phase IN ('prepared', 'applied')
                   AND NOT EXISTS (
                       SELECT 1 FROM effect_reconcile_requests request
                       WHERE request.target_id = attempt.target_id AND request.state = 'claimed'
                         AND request.lease_until > ?1
                   )
                 ORDER BY operation.prepared_at, operation.attempt_id,
                          operation.sequence, operation.id",
            )?;
            let mut rows = statement.query(params![detected_at.millis()])?;
            let mut operations = Vec::new();
            while let Some(row) = rows.next()? {
                operations.push((
                    map_operation(row)?,
                    EffectTargetId::new(row.get::<_, String>("target_id")?),
                ));
            }
            operations
        };
        if operations.is_empty() {
            transaction.commit()?;
            return Ok(0);
        }

        let mut target_recoveries = BTreeMap::new();
        let mut resource_recoveries = BTreeMap::new();
        for (mut operation, target) in operations.iter().cloned() {
            operation
                .require_recovery(detected_at)
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
            update_operation(&transaction, &operation, written_at)?;
            transaction.execute(
                "UPDATE effect_reconcile_attempts
                 SET phase = 'recovery_required', updated_at = MAX(updated_at, ?2)
                 WHERE id = ?1 AND phase NOT IN ('finalized', 'finalized_without_effect')",
                params![operation.attempt().as_str(), written_at.millis()],
            )?;
            transaction.execute(
                "UPDATE effect_operation_artifacts SET state = 'retained', updated_at = MAX(updated_at, ?2)
                 WHERE operation_id = ?1 AND state = 'reserved'",
                params![operation.identity().as_str(), written_at.millis()],
            )?;
            target_recoveries
                .entry(target)
                .or_insert_with(|| (operation.identity().clone(), operation.generation()));
            resource_recoveries
                .entry(operation.resource().clone())
                .or_insert_with(|| (operation.identity().clone(), operation.generation()));
        }

        let mut target_condition_ids = BTreeMap::new();
        for (target, (operation, generation)) in &target_recoveries {
            let mut status = load_target_status(&transaction, target)?.ok_or_else(|| {
                DatabaseError::CorruptEffectState(format!(
                    "recovery operation refers to missing Target status {target}"
                ))
            })?;
            status
                .require_recovery(operation.clone())
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
            save_target_status(&transaction, &status, written_at)?;
            let ids = replace_conditions(
                &transaction,
                &ConditionOwner::Target(target.clone()),
                &[ora_effect::recovery_condition(
                    target,
                    operation,
                    *generation,
                )],
                detected_at,
            )?;
            target_condition_ids.insert(target.clone(), ids);
        }
        for (resource, (operation, generation)) in &resource_recoveries {
            let mut status = load_resource_status(&transaction, resource)?.ok_or_else(|| {
                DatabaseError::CorruptEffectState(format!(
                    "recovery operation refers to missing Resource status {resource}"
                ))
            })?;
            status
                .require_recovery(operation.clone())
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
            save_resource_status(&transaction, &status, written_at)?;
            replace_conditions(
                &transaction,
                &ConditionOwner::Resource(resource.clone()),
                &[ora_effect::resource_recovery_condition(
                    resource,
                    operation,
                    *generation,
                )],
                detected_at,
            )?;
        }
        for (target, condition_ids) in target_condition_ids {
            let updated = transaction.execute(
                "UPDATE effect_reconcile_requests
                 SET state = 'blocked', claim_token = NULL, claim_worker = NULL,
                     lease_until = NULL, retry_attempt = NULL, not_before = NULL,
                     blocked_conditions_json = ?2, resume_trigger_version = 1,
                     resume_trigger_json = '{\"kind\":\"manual_recovery\"}', updated_at = MAX(updated_at, ?3)
                 WHERE target_id = ?1",
                params![
                    target.as_str(),
                    effect_json(&condition_ids)?,
                    written_at.millis(),
                ],
            )?;
            if updated != 1 {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "recovery operation refers to missing Target request {target}"
                )));
            }
            transaction.execute(
                "DELETE FROM effect_resource_claims WHERE target_id = ?1",
                params![target.as_str()],
            )?;
        }
        transaction.commit()?;
        Ok(operations.len())
    })
}

/// Removes artifact authority only when the adapter receipt names the same artifact.
pub(super) fn complete_artifact_cleanup<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    artifact: &ora_effect::ArtifactId,
    receipt: CleanupReceipt,
) -> Result<(), DatabaseError> {
    if receipt.artifact != *artifact {
        return Err(DatabaseError::CorruptEffectState(
            "artifact cleanup receipt identity mismatch".to_string(),
        ));
    }
    repository.pool.with_connection(|connection| {
        let removed = connection.execute(
            "DELETE FROM effect_operation_artifacts WHERE id = ?1 AND state = 'pending_cleanup'",
            params![artifact.as_str()],
        )?;
        if removed != 1 {
            return Err(DatabaseError::CorruptEffectState(format!(
                "artifact {artifact} was not pending cleanup"
            )));
        }
        Ok(())
    })
}

/// Persists cleanup failure without changing already-finalized business state.
pub(super) fn mark_artifact_cleanup_failed<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    artifact: OperationArtifact,
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        let updated = transaction.execute(
            "UPDATE effect_operation_artifacts
             SET state = 'cleanup_failed', updated_at = MAX(updated_at, ?3)
             WHERE id = ?1 AND operation_id = ?2 AND state = 'pending_cleanup'",
            params![
                artifact.identity.as_str(),
                artifact.operation.as_str(),
                written_at.millis(),
            ],
        )?;
        if updated != 1 {
            return Err(DatabaseError::CorruptEffectState(format!(
                "artifact {} was not pending cleanup for operation {}",
                artifact.identity, artifact.operation
            )));
        }
        transaction.commit()?;
        Ok(())
    })
}


/// Atomically closes quarantined journals by exact evidence and reschedules the Target.
///
/// Every supplied closure was proven by a Resource adapter before this transaction opened.
/// The transaction still re-validates durable state: attempts must belong to the Target and
/// still be quarantined, each attempt's operation set must be closed exactly once, and the
/// ownership ledger may only advance for planned-state proofs. A Target whose lifecycle is
/// retiring gets a fresh Scope epoch so its empty retirement projection cannot collide with
/// the immutable projection it persisted while active at the same generation.
pub(super) fn finalize_target_recovery<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    finalization: TargetRecoveryFinalization,
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        let TargetRecoveryFinalization {
            target,
            attempts,
            finalized_at,
        } = &finalization;
        let (lifecycle, scope_id) = transaction.query_row(
            "SELECT lifecycle, scope_id FROM effect_targets WHERE id = ?1",
            params![target.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let mut target_status = load_target_status(&transaction, target)?.ok_or_else(|| {
            DatabaseError::CorruptEffectState(format!("Effect Target {target} has no status"))
        })?;

        let mut affected_resources = BTreeSet::new();
        for closure in attempts {
            let expected_operations: BTreeSet<_> =
                closure.attempt.operations().iter().collect();
            let supplied_operations: BTreeSet<_> = closure
                .operations
                .iter()
                .map(|operation| match operation {
                    ora_effect::RecoveryOperationClosure::Finalized(operation)
                    | ora_effect::RecoveryOperationClosure::ClosedWithoutEffect(operation) => {
                        operation.identity()
                    }
                })
                .collect();
            if expected_operations != supplied_operations {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "recovery closure for attempt {} does not cover its exact operations",
                    closure.attempt.identity()
                )));
            }
            for operation_closure in &closure.operations {
                let (operation, effect_landed) = match operation_closure {
                    ora_effect::RecoveryOperationClosure::Finalized(operation) => (operation, true),
                    ora_effect::RecoveryOperationClosure::ClosedWithoutEffect(operation) => {
                        (operation, false)
                    }
                };
                if operation.attempt() != closure.attempt.identity() {
                    return Err(DatabaseError::CorruptEffectState(
                        "recovery closure mixes operations from another attempt".to_string(),
                    ));
                }
                affected_resources.insert(operation.resource().clone());
                update_operation(&transaction, operation, written_at)?;
                // Only a planned-state proof may advance ownership: a closed-without-effect
                // journal grants nothing because its effect never became durable.
                if effect_landed {
                    apply_recovery_ledger(&transaction, operation, *finalized_at)?;
                }
            }
            let closed = transaction.execute(
                "UPDATE effect_reconcile_attempts
                 SET phase = 'finalized', updated_at = MAX(updated_at, ?3)
                 WHERE id = ?1 AND target_id = ?2 AND phase = 'recovery_required'",
                params![
                    closure.attempt.identity().as_str(),
                    target.as_str(),
                    written_at.millis(),
                ],
            )?;
            if closed != 1 {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "recovery closure for attempt {} was not quarantined",
                    closure.attempt.identity()
                )));
            }
            transaction.execute(
                "UPDATE effect_operation_artifacts
                 SET state = 'pending_cleanup', updated_at = MAX(updated_at, ?3)
                 WHERE operation_id IN (
                     SELECT id FROM effect_operations WHERE attempt_id = ?1
                 )",
                params![
                    closure.attempt.identity().as_str(),
                    target.as_str(),
                    written_at.millis(),
                ],
            )?;
        }

        // The Target leaves manual recovery only here: its watermarks stay untouched because
        // readiness may advance solely through a fresh reconcile against current Desired State.
        if lifecycle == "retiring" {
            target_status
                .resume_retiring_from_recovery()
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        } else {
            target_status
                .resume_from_recovery()
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        }
        save_recovered_target_status(&transaction, &target_status, written_at)?;
        replace_conditions(
            &transaction,
            &ConditionOwner::Target(target.clone()),
            &[],
            written_at,
        )?;
        for resource in &affected_resources {
            resume_recovered_resource(&transaction, resource, written_at)?;
        }

        let generation = generation_from_sql(transaction.query_row(
            "SELECT generation FROM effect_scopes WHERE id = ?1",
            params![&scope_id],
            |row| row.get::<_, i64>(0),
        )?)?;
        if lifecycle == "retiring" {
            // A retiring Target converges to an empty projection; without a fresh epoch that
            // empty projection would collide with the immutable non-empty projection the
            // Target persisted while active at the same generation (issue #6 legacy states).
            // The advance also wakes every Scope Target, including this one, at the new epoch.
            advance_topology_epochs(
                &transaction,
                &BTreeSet::from([scope_id.clone()]),
                written_at.millis(),
            )?;
        } else {
            super::declaration::upsert_target_wakeup(
                &transaction,
                target.as_str(),
                generation,
                written_at.millis(),
                "user_requested",
                written_at.millis(),
            )?;
        }
        let finalized_operations = attempts
            .iter()
            .flat_map(|closure| closure.operations.iter())
            .filter(|operation| {
                matches!(
                    operation,
                    ora_effect::RecoveryOperationClosure::Finalized(_)
                )
            })
            .count();
        let closed_without_effect_operations = attempts
            .iter()
            .flat_map(|closure| closure.operations.iter())
            .filter(|operation| {
                matches!(
                    operation,
                    ora_effect::RecoveryOperationClosure::ClosedWithoutEffect(_)
                )
            })
            .count();
        transaction.execute(
            "INSERT INTO effect_audit_events (
                 id, scope_id, subject_kind, subject_id, event_kind, generation,
                 initiator_kind, initiator_id, payload_version, payload_json, occurred_at
             ) VALUES (?1, ?2, 'target', ?3, 'recovery_finalized', ?4,
                       'user', NULL, 1, ?5, ?6)",
            params![
                Uuid::new_v4().to_string(),
                &scope_id,
                target.as_str(),
                generation_to_sql(generation)?,
                effect_json(&serde_json::json!({
                    "finalized_operations": finalized_operations,
                    "closed_without_effect_operations": closed_without_effect_operations,
                }))?,
                written_at.millis(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    })
}

/// Advances the ownership ledger only for a planned-state proof; a closed-without-effect
/// journal grants nothing because its effect never became durable.
fn apply_recovery_ledger(
    transaction: &Transaction<'_>,
    operation: &EffectOperation,
    finalized_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    match operation.planned() {
        ora_effect::ExactPlannedState::Present {
            native_identity,
            fingerprint,
            managed_identity,
        } => {
            let (desired_effect, revision) = transaction.query_row(
                "SELECT materialization.desired_effect_id, materialization.revision_id
                 FROM effect_attempt_resource_projections attempt_projection
                 JOIN effect_resolved_materializations materialization
                   ON materialization.projection_digest = attempt_projection.resource_projection_digest
                 WHERE attempt_projection.attempt_id = ?1
                   AND materialization.managed_identity = ?2",
                params![operation.attempt().as_str(), managed_identity.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            transaction.execute(
                "INSERT INTO effect_managed_items (
                     id, scope_id, resource_id, desired_effect_id, applied_revision_id,
                     native_identity, fingerprint, applied_generation, created_at, updated_at
                 ) SELECT ?1, resource.scope_id, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8
                   FROM effect_resources resource WHERE resource.id = ?2
                 ON CONFLICT(id) DO UPDATE SET
                     desired_effect_id = excluded.desired_effect_id,
                     applied_revision_id = excluded.applied_revision_id,
                     native_identity = excluded.native_identity, fingerprint = excluded.fingerprint,
                     applied_generation = excluded.applied_generation, updated_at = MAX(effect_managed_items.updated_at, excluded.updated_at)",
                params![
                    managed_identity.as_str(),
                    operation.resource().as_str(),
                    desired_effect,
                    revision,
                    native_identity.as_str(),
                    fingerprint.as_str(),
                    generation_to_sql(operation.generation())?,
                    finalized_at.millis(),
                ],
            )?;
            Ok(())
        }
        ora_effect::ExactPlannedState::Missing => {
            let ora_effect::ExactPreviousState::Present { managed_identity, .. } =
                operation.expected()
            else {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "delete operation {} lacks previous ownership",
                    operation.identity()
                )));
            };
            transaction.execute(
                "DELETE FROM effect_managed_items WHERE id = ?1",
                params![managed_identity.as_str()],
            )?;
            Ok(())
        }
    }
}

/// Writes a Target status that explicitly left manual recovery, clearing its quarantine link.
fn save_recovered_target_status(
    transaction: &Transaction<'_>,
    status: &ora_effect::TargetStatus,
    written_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let progress = status.progress();
    let phase = match status.phase() {
        ora_effect::TargetPhase::Pending => "pending",
        ora_effect::TargetPhase::Retiring => "retiring",
        other => {
            return Err(DatabaseError::CorruptEffectState(format!(
                "recovered Target status keeps phase {other:?}"
            )));
        }
    };
    transaction.execute(
        "UPDATE effect_target_status SET
             desired_generation = MAX(desired_generation, ?2),
             observed_generation = MAX(observed_generation, ?3),
             applied_generation = MAX(applied_generation, ?4),
             ready_generation = MAX(ready_generation, ?5),
             phase = ?6, recovery_operation_id = NULL,
             status_version = MAX(status_version + 1, ?7),
             updated_at = MAX(updated_at, ?8)
         WHERE target_id = ?1 AND phase = 'recovery_required'",
        params![
            status.target().as_str(),
            generation_to_sql(progress.desired())?,
            generation_to_sql(progress.observed())?,
            generation_to_sql(progress.applied())?,
            generation_to_sql(progress.ready())?,
            phase,
            super::mapping::status_version_to_sql(status.version())?,
            written_at.millis(),
        ],
    )?;
    Ok(())
}

/// Resumes a Resource unless another Target's quarantined operation still blocks it.
fn resume_recovered_resource(
    transaction: &Transaction<'_>,
    resource: &ora_effect::EffectResourceId,
    written_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let remaining = transaction
        .query_row(
            "SELECT id, generation FROM effect_operations
             WHERE resource_id = ?1 AND phase = 'recovery_required'
             ORDER BY prepared_at, id LIMIT 1",
            params![resource.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    match remaining {
        None => {
            let mut status = load_resource_status(transaction, resource)?.ok_or_else(|| {
                DatabaseError::CorruptEffectState(format!(
                    "recovery refers to missing Resource status {resource}"
                ))
            })?;
            status
                .resume_from_recovery()
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
            save_resource_status(transaction, &status, written_at)?;
            replace_conditions(
                transaction,
                &ConditionOwner::Resource(resource.clone()),
                &[],
                written_at,
            )?;
        }
        Some((operation, generation)) => {
            // Another Target still owes recovery on this shared Resource: keep the
            // quarantine, but re-point its evidence at an operation that still exists.
            transaction.execute(
                "UPDATE effect_resource_status
                 SET recovery_operation_id = ?2, status_version = status_version + 1,
                     updated_at = MAX(updated_at, ?3)
                 WHERE resource_id = ?1 AND phase = 'recovery_required'",
                params![resource.as_str(), &operation, written_at.millis()],
            )?;
            replace_conditions(
                transaction,
                &ConditionOwner::Resource(resource.clone()),
                &[ora_effect::resource_recovery_condition(
                    resource,
                    &ora_effect::EffectOperationId::new(operation),
                    generation_from_sql(generation)?,
                )],
                written_at,
            )?;
        }
    }
    Ok(())
}
