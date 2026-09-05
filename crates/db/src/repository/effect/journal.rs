use super::EffectWriteContext;
use super::SqliteEffectRepository;
use super::claims::{release_resource_claims, verify_claim, verify_resource_claims};
use super::ledger_validation::{
    validate_attempt_managed_transition, validate_operation_managed_evidence,
};
use super::mapping::{effect_json, generation_to_sql};
use super::operations::{insert_operation, update_operation};
use super::persistence::{
    attempt_phase_value, attempt_progress_can_advance, complete_request, finish_retiring_target,
    insert_artifact, insert_coordination_receipt, insert_readiness, parse_attempt_phase,
    save_current_state,
};
use super::projection_persistence::save_projections;
use super::validation::{validate_current_state_scope, validate_projection_scope};
use crate::DatabaseError;
use crate::TimestampSource;
use ora_effect::{
    AttemptFinalization, CoordinationReceipt, CoordinationReceiptState, CoordinationRequirement,
    EffectOperation, EffectResourceId, OperationArtifact, OperationProgress, ReconcileAttempt,
    ReconcileAttemptPhase, ReconcileClaim, TargetProjection,
};
use rusqlite::{TransactionBehavior, params};
use std::collections::BTreeSet;

/// Persists immutable projections, Attempt, Operations, and Artifacts before external effects.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_attempt<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    claim: &ReconcileClaim,
    attempt: ReconcileAttempt,
    target_projections: Vec<TargetProjection>,
    resource_projections: Vec<ora_effect::ResourceProjection>,
    operations: Vec<EffectOperation>,
    artifacts: Vec<OperationArtifact>,
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        verify_claim(&transaction, attempt.target(), claim)?;
        let prepared_at = operations
            .first()
            .and_then(|operation| match operation.progress() {
                OperationProgress::Prepared { prepared_at } => Some(*prepared_at),
                OperationProgress::Applied { .. }
                | OperationProgress::Finalized { .. }
                | OperationProgress::RecoveryRequired { .. } => None,
            })
            .ok_or_else(|| {
                DatabaseError::CorruptEffectState(
                    "a prepared Effect attempt requires Prepared operations".to_string(),
                )
            })?;
        let operation_ids = operations
            .iter()
            .map(|operation| operation.identity().clone())
            .collect::<Vec<_>>();
        if attempt.phase() != ReconcileAttemptPhase::Prepared
            || attempt.operations() != operation_ids
            || operations.iter().any(|operation| {
                operation.attempt() != attempt.identity()
                    || operation.generation() != attempt.generation()
                    || !matches!(
                        operation.progress(),
                        OperationProgress::Prepared {
                            prepared_at: operation_prepared_at
                        } if *operation_prepared_at == prepared_at
                    )
            })
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt journal input does not match its operations".to_string(),
            ));
        }
        let target_revision = transaction.query_row(
            "SELECT consumer_revision_id FROM effect_targets WHERE id = ?1",
            params![attempt.target().as_str()],
            |row| row.get::<_, String>(0),
        )?;
        if target_revision != attempt.consumer_revision().as_str() {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt Consumer Revision does not match its Target".to_string(),
            ));
        }
        let claimed_projection = target_projections
            .iter()
            .find(|projection| projection.target == *attempt.target());
        if claimed_projection.is_none_or(|projection| {
            projection.digest != *attempt.target_projection()
                || projection.consumer_revision != *attempt.consumer_revision()
                || projection.generation != attempt.generation()
        }) {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt lacks its exact Target projection".to_string(),
            ));
        }
        let projected_targets = target_projections
            .iter()
            .map(|projection| projection.target.clone())
            .collect::<BTreeSet<_>>();
        if attempt
            .coordination()
            .participants
            .keys()
            .any(|target| !projected_targets.contains(target))
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect Attempt coordinates a Target outside its persisted projections".to_string(),
            ));
        }
        let projected_resources = validate_projection_scope(
            &transaction,
            attempt.target(),
            attempt.generation(),
            &target_projections,
            &resource_projections,
        )?;
        verify_resource_claims(&transaction, attempt.target(), claim, &projected_resources)?;
        let supplied_resource_digests = resource_projections
            .iter()
            .map(|projection| projection.digest.clone())
            .collect::<BTreeSet<_>>();
        if supplied_resource_digests.len() != resource_projections.len()
            || supplied_resource_digests != *attempt.resource_projections()
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt Resource projection set is incomplete".to_string(),
            ));
        }
        let operation_resources = operations
            .iter()
            .map(|operation| operation.resource().clone())
            .collect::<BTreeSet<_>>();
        if !attempt
            .coordination()
            .resources
            .is_subset(&projected_resources)
            || operation_resources != attempt.coordination().resources
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt mutation authority does not match its Resource projections"
                    .to_string(),
            ));
        }
        let operation_id_set = operation_ids.iter().collect::<BTreeSet<_>>();
        if artifacts
            .iter()
            .any(|artifact| !operation_id_set.contains(&artifact.operation))
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect Artifact refers outside its prepared Attempt".to_string(),
            ));
        }
        save_projections(
            &transaction,
            &target_projections,
            &resource_projections,
            written_at,
        )?;
        transaction.execute(
            "INSERT INTO effect_reconcile_attempts (
                 id, target_id, generation, consumer_revision_id, target_projection_digest,
                 coordination_plan_version, coordination_plan_json, phase, prepared_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 'prepared', ?7, ?8)",
            params![
                attempt.identity().as_str(),
                attempt.target().as_str(),
                generation_to_sql(attempt.generation())?,
                attempt.consumer_revision().as_str(),
                attempt.target_projection().digest().as_str(),
                effect_json(attempt.coordination())?,
                prepared_at.millis(),
                written_at.max(prepared_at).millis(),
            ],
        )?;
        let mut projection_digests = attempt.resource_projections().iter().collect::<Vec<_>>();
        projection_digests.sort();
        for (sequence, digest) in projection_digests.into_iter().enumerate() {
            transaction.execute(
                "INSERT INTO effect_attempt_resource_projections (
                     attempt_id, resource_projection_digest, sequence
                 ) VALUES (?1, ?2, ?3)",
                params![
                    attempt.identity().as_str(),
                    digest.digest().as_str(),
                    i64::try_from(sequence).map_err(|_| {
                        DatabaseError::CorruptEffectState(
                            "Resource projection sequence exceeds SQLite INTEGER".to_string(),
                        )
                    })?,
                ],
            )?;
        }
        for operation in &operations {
            insert_operation(&transaction, operation, written_at)?;
        }
        for artifact in &artifacts {
            insert_artifact(&transaction, artifact, written_at)?;
        }
        transaction.commit()?;
        Ok(())
    })
}

/// Persists every externally proven attempt step so a crash never erases known progress.
pub(super) fn record_attempt_progress<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    claim: &ReconcileClaim,
    attempt: &ReconcileAttempt,
    operations: &[EffectOperation],
    receipts: &[ora_effect::CoordinationReceipt],
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        verify_claim(&transaction, attempt.target(), claim)?;
        let stored_phase = transaction.query_row(
            "SELECT phase FROM effect_reconcile_attempts WHERE id = ?1 AND target_id = ?2",
            params![attempt.identity().as_str(), attempt.target().as_str()],
            |row| row.get::<_, String>(0),
        )?;
        let stored_phase = parse_attempt_phase(&stored_phase)?;
        if !attempt_progress_can_advance(stored_phase, attempt.phase())
            || matches!(
                attempt.phase(),
                ReconcileAttemptPhase::Finalized | ReconcileAttemptPhase::RecoveryRequired
            )
        {
            return Err(DatabaseError::CorruptEffectState(format!(
                "illegal Effect attempt progress {:?} -> {:?}",
                stored_phase,
                attempt.phase()
            )));
        }
        let expected_operations = attempt.operations().iter().collect::<BTreeSet<_>>();
        let supplied_operations = operations
            .iter()
            .map(EffectOperation::identity)
            .collect::<BTreeSet<_>>();
        if expected_operations != supplied_operations
            || operations
                .iter()
                .any(|operation| operation.attempt() != attempt.identity())
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt progress contains mismatched operations".to_string(),
            ));
        }
        let operations_must_be_applied = matches!(
            attempt.phase(),
            ReconcileAttemptPhase::Applied
                | ReconcileAttemptPhase::Verified
                | ReconcileAttemptPhase::Activated
        );
        if operations_must_be_applied
            && operations
                .iter()
                .any(|operation| !matches!(operation.progress(), OperationProgress::Applied { .. }))
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect Attempt phase exceeds its Operation evidence".to_string(),
            ));
        }
        validate_coordination_receipts(attempt, receipts)?;
        for operation in operations {
            update_operation(&transaction, operation, written_at)?;
        }
        for receipt in receipts {
            insert_coordination_receipt(&transaction, attempt, receipt, written_at)?;
        }
        transaction.execute(
            "UPDATE effect_reconcile_attempts SET phase = ?2, updated_at = MAX(updated_at, ?3) WHERE id = ?1",
            params![
                attempt.identity().as_str(),
                attempt_phase_value(attempt.phase()),
                written_at.millis(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    })
}

/// Atomically finalizes journal phase, ledgers, receipts, statuses, and the Target request.
pub(super) fn finalize_attempt<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    claim: &ReconcileClaim,
    finalization: AttemptFinalization,
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        verify_claim(&transaction, finalization.attempt.target(), claim)?;
        if finalization.attempt.phase() != ReconcileAttemptPhase::Finalized {
            return Err(DatabaseError::CorruptEffectState(
                "Effect finalization requires a Finalized attempt".to_string(),
            ));
        }
        let target_status = finalization
            .target_statuses
            .iter()
            .find(|status| status.target() == finalization.attempt.target())
            .ok_or_else(|| {
                DatabaseError::CorruptEffectState(
                    "finalization lacks claimed Target status".to_string(),
                )
            })?;
        if finalization.target_statuses.len() != 1
            || target_status.progress().ready() != finalization.attempt.generation()
            || finalization.readiness.as_ref().is_none_or(|readiness| {
                readiness.target != *finalization.attempt.target()
                    || readiness.generation != finalization.attempt.generation()
                    || readiness.consumer_revision != *finalization.attempt.consumer_revision()
                    || readiness.projection != *finalization.attempt.target_projection()
            })
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect finalization lacks exact Target readiness evidence".to_string(),
            ));
        }
        let finalized_at = written_at;
        let expected_resources = {
            let mut statement = transaction.prepare(
                "SELECT projection.resource_id
                 FROM effect_attempt_resource_projections attempt_projection
                 JOIN effect_resource_projections projection
                   ON projection.digest = attempt_projection.resource_projection_digest
                 WHERE attempt_projection.attempt_id = ?1",
            )?;
            statement
                .query_map(params![finalization.attempt.identity().as_str()], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(EffectResourceId::new)
                .collect::<BTreeSet<_>>()
        };
        let status_resources = finalization
            .resource_statuses
            .iter()
            .map(|status| status.resource().clone())
            .collect::<BTreeSet<_>>();
        if status_resources != expected_resources
            || finalization
                .resource_statuses
                .iter()
                .any(|status| status.applied() != finalization.attempt.generation())
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect finalization Resource statuses do not match its projections".to_string(),
            ));
        }
        verify_resource_claims(
            &transaction,
            finalization.attempt.target(),
            claim,
            &expected_resources,
        )?;
        validate_current_state_scope(
            &transaction,
            finalization.attempt.target(),
            finalization.attempt.generation(),
            &expected_resources,
            &finalization.target_statuses,
            &finalization.resource_statuses,
            &finalization.managed,
            &finalization.removed_managed,
            &finalization.conditions,
        )?;
        validate_attempt_managed_transition(
            &transaction,
            finalization.attempt.identity(),
            &finalization.managed,
            &finalization.removed_managed,
        )?;
        validate_operation_managed_evidence(
            &finalization.operations,
            &finalization.managed,
            &finalization.removed_managed,
        )?;
        let expected_operations = finalization
            .attempt
            .operations()
            .iter()
            .collect::<BTreeSet<_>>();
        let supplied_operations = finalization
            .operations
            .iter()
            .map(EffectOperation::identity)
            .collect::<BTreeSet<_>>();
        if expected_operations != supplied_operations
            || finalization.operations.iter().any(|operation| {
                operation.attempt() != finalization.attempt.identity()
                    || !matches!(operation.progress(), OperationProgress::Finalized { .. })
            })
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect finalization contains mismatched operation journals".to_string(),
            ));
        }
        validate_coordination_receipts(&finalization.attempt, &finalization.coordination_receipts)?;
        for operation in &finalization.operations {
            update_operation(&transaction, operation, written_at)?;
        }
        let attempt_updated = transaction.execute(
            "UPDATE effect_reconcile_attempts SET phase = 'finalized', updated_at = MAX(updated_at, ?2)
             WHERE id = ?1 AND phase = 'activated'",
            params![
                finalization.attempt.identity().as_str(),
                finalized_at.millis(),
            ],
        )?;
        if attempt_updated != 1 {
            return Err(DatabaseError::CorruptEffectState(
                "Effect attempt was not Activated before finalization".to_string(),
            ));
        }
        for receipt in &finalization.coordination_receipts {
            insert_coordination_receipt(
                &transaction,
                &finalization.attempt,
                receipt,
                finalized_at,
            )?;
        }
        save_current_state(
            &transaction,
            &finalization.managed,
            &finalization.removed_managed,
            &finalization.target_statuses,
            &finalization.resource_statuses,
            &finalization.conditions,
            finalized_at,
        )?;
        if let Some(readiness) = &finalization.readiness {
            insert_readiness(&transaction, readiness, finalized_at)?;
        }
        transaction.execute(
            "UPDATE effect_operation_artifacts SET state = 'pending_cleanup', updated_at = MAX(updated_at, ?2)
             WHERE operation_id IN (
                 SELECT id FROM effect_operations WHERE attempt_id = ?1
             )",
            params![
                finalization.attempt.identity().as_str(),
                finalized_at.millis(),
            ],
        )?;
        complete_request(
            &transaction,
            finalization.attempt.target(),
            claim,
            target_status.progress().ready(),
            written_at,
        )?;
        release_resource_claims(&transaction, finalization.attempt.target(), claim)?;
        finish_retiring_target(&transaction, finalization.attempt.target())?;
        transaction.commit()?;
        Ok(())
    })
}

/// Validates exact participant contracts and required barriers for the Attempt's current phase.
fn validate_coordination_receipts(
    attempt: &ReconcileAttempt,
    receipts: &[CoordinationReceipt],
) -> Result<(), DatabaseError> {
    let required_targets = attempt
        .coordination()
        .participants
        .iter()
        .filter_map(|(target, requirement)| match requirement {
            CoordinationRequirement::Uninterrupted => None,
            CoordinationRequirement::QuiesceBeforeMutation(_) => Some(target.clone()),
        })
        .collect::<BTreeSet<_>>();
    let mut safe_targets = BTreeSet::new();
    let mut reactivated_targets = BTreeSet::new();
    for receipt in receipts {
        let Some(CoordinationRequirement::QuiesceBeforeMutation(contract)) =
            attempt.coordination().participants.get(&receipt.target)
        else {
            return Err(DatabaseError::CorruptEffectState(format!(
                "coordination receipt names undeclared Target {}",
                receipt.target
            )));
        };
        if receipt.contract != *contract {
            return Err(DatabaseError::CorruptEffectState(format!(
                "coordination receipt contract mismatches Target {}",
                receipt.target
            )));
        }
        let inserted = match receipt.state {
            CoordinationReceiptState::SafeToMutate => safe_targets.insert(receipt.target.clone()),
            CoordinationReceiptState::Reactivated => {
                reactivated_targets.insert(receipt.target.clone())
            }
        };
        if !inserted {
            return Err(DatabaseError::CorruptEffectState(format!(
                "duplicate coordination receipt for Target {}",
                receipt.target
            )));
        }
    }
    let safe_required = !matches!(attempt.phase(), ReconcileAttemptPhase::Prepared);
    let reactivation_required = matches!(
        attempt.phase(),
        ReconcileAttemptPhase::Activated | ReconcileAttemptPhase::Finalized
    );
    if (safe_required && safe_targets != required_targets)
        || (reactivation_required && reactivated_targets != required_targets)
        || (!reactivation_required && !reactivated_targets.is_empty())
    {
        return Err(DatabaseError::CorruptEffectState(
            "Effect Attempt phase lacks its exact coordination barriers".to_string(),
        ));
    }
    Ok(())
}
