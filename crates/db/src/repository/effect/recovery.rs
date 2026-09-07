use super::EffectWriteContext;
use super::SqliteEffectRepository;
use super::conditions::replace_conditions;
use super::mapping::{effect_json, load_resource_status, load_target_status};
use super::operations::{map_operation, update_operation};
use super::store::{save_resource_status, save_target_status};
use crate::DatabaseError;
use crate::TimestampSource;
use ora_effect::{
    CleanupReceipt, ConditionOwner, EffectOperation, EffectTargetId, LocalTimestamp,
    OperationArtifact,
};
use rusqlite::{TransactionBehavior, params};
use std::collections::BTreeMap;

/// Loads unfinished immutable operation journals in deterministic preparation order.
pub(super) fn load_unfinished_operations<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
) -> Result<Vec<EffectOperation>, DatabaseError> {
    repository.pool.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT id, attempt_id, resource_id, generation, sequence, mutation,
                    expected_json, planned_json, payload_json, phase,
                    prepared_at, applied_at, finalized_at, updated_at, detected_at
             FROM effect_operations WHERE phase <> 'finalized'
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
                 WHERE id = ?1 AND phase <> 'finalized'",
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
