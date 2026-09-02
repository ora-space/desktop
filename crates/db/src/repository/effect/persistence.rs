use super::mapping::{
    effect_json, generation_from_sql, generation_to_sql, mutation_value, parse_effect_json,
    parse_mutation,
};
use super::store::{replace_conditions, save_resource_status, save_target_status};
use crate::DatabaseError;
use ora_effect::{
    ArtifactRole, ArtifactState, ConditionOwner, ConditionProposal, CoordinationReceiptState,
    EffectOperation, EffectOperationId, EffectOperationIntent, EffectResourceId, EffectTargetId,
    Generation, LocalTimestamp, ManagedIdentity, ManagedItem, OperationArtifact, OperationProgress,
    ReconcileAttempt, ReconcileAttemptPhase, ReconcileClaim, ResourceStatus,
};
use rusqlite::{Transaction, params};
use std::collections::BTreeMap;

/// Groups deterministic Condition proposals by the status owner they replace.
pub(super) fn group_conditions(
    conditions: Vec<ConditionProposal>,
) -> BTreeMap<ConditionOwner, Vec<ConditionProposal>> {
    let mut grouped = BTreeMap::new();
    for condition in conditions {
        grouped
            .entry(condition.owner.clone())
            .or_insert_with(Vec::new)
            .push(condition);
    }
    grouped
}

/// Persists current ledgers/statuses/Conditions as one validated business transition.
pub(super) fn save_current_state(
    transaction: &Transaction<'_>,
    managed: &[ManagedItem],
    removed_managed: &[ManagedIdentity],
    target_statuses: &[ora_effect::TargetStatus],
    resource_statuses: &[ResourceStatus],
    conditions: &[ConditionProposal],
    updated_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    for identity in removed_managed {
        transaction.execute(
            "DELETE FROM effect_managed_items WHERE id = ?1",
            params![identity.as_str()],
        )?;
    }
    for item in managed {
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
                 applied_generation = excluded.applied_generation, updated_at = excluded.updated_at",
            params![
                item.identity.as_str(),
                item.resource.as_str(),
                item.desired_effect.as_str(),
                item.applied_revision.as_str(),
                item.native_identity.as_str(),
                item.fingerprint.as_str(),
                generation_to_sql(item.applied_generation)?,
                updated_at.millis(),
            ],
        )?;
    }
    for status in target_statuses {
        save_target_status(transaction, status)?;
    }
    for status in resource_statuses {
        save_resource_status(transaction, status)?;
    }
    let grouped = group_conditions(conditions.to_vec());
    for status in target_statuses {
        let owner = ConditionOwner::Target(status.target().clone());
        replace_conditions(
            transaction,
            &owner,
            grouped.get(&owner).map(Vec::as_slice).unwrap_or_default(),
            updated_at,
        )?;
    }
    for status in resource_statuses {
        let owner = ConditionOwner::Resource(status.resource().clone());
        replace_conditions(
            transaction,
            &owner,
            grouped.get(&owner).map(Vec::as_slice).unwrap_or_default(),
            updated_at,
        )?;
    }
    Ok(())
}

/// Inserts a Readiness Receipt tied to its exact Consumer Revision and projection digest.
pub(super) fn insert_readiness(
    transaction: &Transaction<'_>,
    readiness: &ora_effect::ReadinessReceipt,
    received_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let generation = generation_to_sql(readiness.generation)?;
    let proof_version = i64::from(readiness.proof.version);
    let proof_json = effect_json(&readiness.proof.payload)?;
    let inserted = transaction.execute(
        "INSERT INTO effect_readiness_receipts (
             target_id, generation, consumer_revision_id, projection_digest,
             proof_version, proof_json, received_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(target_id, generation, consumer_revision_id, projection_digest)
         DO NOTHING",
        params![
            readiness.target.as_str(),
            generation,
            readiness.consumer_revision.as_str(),
            readiness.projection.digest().as_str(),
            proof_version,
            &proof_json,
            received_at.millis(),
        ],
    )?;
    if inserted == 0 {
        let exact = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM effect_readiness_receipts
                 WHERE target_id = ?1 AND generation = ?2 AND consumer_revision_id = ?3
                   AND projection_digest = ?4 AND proof_version = ?5 AND proof_json = ?6
             )",
            params![
                readiness.target.as_str(),
                generation,
                readiness.consumer_revision.as_str(),
                readiness.projection.digest().as_str(),
                proof_version,
                proof_json,
            ],
            |row| row.get::<_, i64>(0),
        )? != 0;
        if !exact {
            return Err(DatabaseError::CorruptEffectState(
                "conflicting immutable Effect readiness receipt".to_string(),
            ));
        }
    }
    Ok(())
}

/// Completes a request only through the generation proven ready, preserving newer work.
pub(super) fn complete_request(
    transaction: &Transaction<'_>,
    target: &EffectTargetId,
    claim: &ReconcileClaim,
    ready: Generation,
    updated_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let requested = transaction.query_row(
        "SELECT requested_generation FROM effect_reconcile_requests
         WHERE target_id = ?1 AND state = 'claimed' AND claim_token = ?2",
        params![
            target.as_str(),
            i64::try_from(claim.token.value()).map_err(|_| {
                DatabaseError::CorruptEffectState(
                    "Target fencing token exceeds SQLite INTEGER".to_string(),
                )
            })?,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if generation_from_sql(requested)? <= ready {
        transaction.execute(
            "DELETE FROM effect_reconcile_requests
             WHERE target_id = ?1 AND state = 'claimed' AND claim_token = ?2",
            params![
                target.as_str(),
                i64::try_from(claim.token.value()).map_err(|_| {
                    DatabaseError::CorruptEffectState(
                        "Target fencing token exceeds SQLite INTEGER".to_string(),
                    )
                })?,
            ],
        )?;
    } else {
        transaction.execute(
            "UPDATE effect_reconcile_requests
             SET state = 'pending', claim_token = NULL, claim_worker = NULL, lease_until = NULL,
                 retry_count = 0, retry_attempt = NULL, not_before = NULL,
                 blocked_conditions_json = NULL, resume_trigger_version = NULL,
                 resume_trigger_json = NULL, updated_at = MAX(updated_at, ?3)
             WHERE target_id = ?1 AND claim_token = ?2",
            params![
                target.as_str(),
                i64::try_from(claim.token.value()).map_err(|_| {
                    DatabaseError::CorruptEffectState(
                        "Target fencing token exceeds SQLite INTEGER".to_string(),
                    )
                })?,
                updated_at.millis(),
            ],
        )?;
    }
    Ok(())
}

/// Deletes a retiring Target after its empty contribution has been successfully reconciled.
pub(super) fn finish_retiring_target(
    transaction: &Transaction<'_>,
    target: &EffectTargetId,
) -> Result<(), DatabaseError> {
    let retiring = transaction.query_row(
        "SELECT lifecycle = 'retiring' FROM effect_targets WHERE id = ?1",
        params![target.as_str()],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if !retiring {
        return Ok(());
    }
    // Shared Resource ledger rows prove surviving projections, so they cannot keep the empty
    // retiring Target identity alive after this reconcile has completed.
    let mut statement = transaction
        .prepare("SELECT resource_id FROM effect_target_resource_bindings WHERE target_id = ?1")?;
    let resources = statement
        .query_map(params![target.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    transaction.execute(
        "DELETE FROM effect_targets WHERE id = ?1",
        params![target.as_str()],
    )?;
    for resource in resources {
        let bindings = transaction.query_row(
            "SELECT count(*) FROM effect_target_resource_bindings WHERE resource_id = ?1",
            params![&resource],
            |row| row.get::<_, i64>(0),
        )?;
        if bindings == 0 {
            transaction.execute(
                "DELETE FROM effect_resources WHERE id = ?1
                 AND NOT EXISTS (
                     SELECT 1 FROM effect_managed_items WHERE resource_id = ?1
                 )",
                params![&resource],
            )?;
        }
    }
    Ok(())
}

/// Inserts one immutable operation intent and its initial Prepared progress.
pub(super) fn insert_operation(
    transaction: &Transaction<'_>,
    operation: &EffectOperation,
) -> Result<(), DatabaseError> {
    let OperationProgress::Prepared { prepared_at } = operation.progress() else {
        return Err(DatabaseError::CorruptEffectState(
            "new Effect operation is not Prepared".to_string(),
        ));
    };
    transaction.execute(
        "INSERT INTO effect_operations (
             id, attempt_id, resource_id, generation, sequence, mutation,
             expected_version, expected_json, planned_version, planned_json,
             payload_version, payload_json, phase, prepared_at, applied_at,
             finalized_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, 1, ?8, 1, ?9,
                   'prepared', ?10, NULL, NULL, ?10)",
        params![
            operation.identity().as_str(),
            operation.attempt().as_str(),
            operation.resource().as_str(),
            generation_to_sql(operation.generation())?,
            i64::from(operation.sequence()),
            mutation_value(operation.mutation()),
            effect_json(operation.expected())?,
            effect_json(operation.planned())?,
            effect_json(operation.payload())?,
            prepared_at.millis(),
        ],
    )?;
    Ok(())
}

/// Updates only legal operation progress columns; immutable intent is protected by a trigger.
pub(super) fn update_operation(
    transaction: &Transaction<'_>,
    operation: &EffectOperation,
) -> Result<(), DatabaseError> {
    let (phase, prepared_at, applied_at, finalized_at, updated_at) = match operation.progress() {
        OperationProgress::Prepared { prepared_at } => {
            ("prepared", *prepared_at, None, None, *prepared_at)
        }
        OperationProgress::Applied {
            prepared_at,
            applied_at,
        } => (
            "applied",
            *prepared_at,
            Some(*applied_at),
            None,
            *applied_at,
        ),
        OperationProgress::Finalized {
            prepared_at,
            applied_at,
            finalized_at,
        } => (
            "finalized",
            *prepared_at,
            Some(*applied_at),
            Some(*finalized_at),
            *finalized_at,
        ),
        OperationProgress::RecoveryRequired {
            prepared_at,
            applied_at,
            detected_at,
        } => (
            "recovery_required",
            *prepared_at,
            *applied_at,
            None,
            *detected_at,
        ),
    };
    let stored_phase = transaction.query_row(
        "SELECT phase FROM effect_operations WHERE id = ?1 AND attempt_id = ?2",
        params![operation.identity().as_str(), operation.attempt().as_str()],
        |row| row.get::<_, String>(0),
    )?;
    if !operation_progress_can_advance(&stored_phase, phase) {
        return Err(DatabaseError::CorruptEffectState(format!(
            "illegal Effect operation progress {stored_phase} -> {phase}"
        )));
    }
    let updated = transaction.execute(
        "UPDATE effect_operations
         SET phase = ?2, applied_at = ?3, finalized_at = ?4, updated_at = ?5
         WHERE id = ?1 AND prepared_at = ?6 AND attempt_id = ?7",
        params![
            operation.identity().as_str(),
            phase,
            applied_at.map(LocalTimestamp::millis),
            finalized_at.map(LocalTimestamp::millis),
            updated_at.millis(),
            prepared_at.millis(),
            operation.attempt().as_str(),
        ],
    )?;
    if updated != 1 {
        return Err(DatabaseError::CorruptEffectState(
            "Effect operation progress did not match immutable journal input".to_string(),
        ));
    }
    Ok(())
}

/// Inserts exact operation-owned artifact authority before its first possible creation.
pub(super) fn insert_artifact(
    transaction: &Transaction<'_>,
    artifact: &OperationArtifact,
    created_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_operation_artifacts (
             id, operation_id, role, locator_version, locator_json,
             expected_fingerprint, state, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?7)",
        params![
            artifact.identity.as_str(),
            artifact.operation.as_str(),
            artifact_role_value(artifact.role),
            effect_json(&artifact.locator)?,
            artifact.expected_fingerprint.as_str(),
            artifact_state_value(artifact.state),
            created_at.millis(),
        ],
    )?;
    Ok(())
}

/// Persists one coordination proof tied to the immutable attempt.
pub(super) fn insert_coordination_receipt(
    transaction: &Transaction<'_>,
    attempt: &ReconcileAttempt,
    receipt: &ora_effect::CoordinationReceipt,
    received_at: LocalTimestamp,
) -> Result<(), DatabaseError> {
    let contract_version = i64::from(receipt.contract.version);
    let contract_json = effect_json(&receipt.contract)?;
    let state = match receipt.state {
        CoordinationReceiptState::SafeToMutate => "safe_to_mutate",
        CoordinationReceiptState::Reactivated => "reactivated",
    };
    let proof_version = i64::from(receipt.proof.version);
    let proof_json = effect_json(&receipt.proof.payload)?;
    let inserted = transaction.execute(
        "INSERT INTO effect_coordination_receipts (
             attempt_id, target_id, contract_version, contract_json, state,
             proof_version, proof_json, received_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(attempt_id, target_id, state) DO NOTHING",
        params![
            attempt.identity().as_str(),
            receipt.target.as_str(),
            contract_version,
            &contract_json,
            state,
            proof_version,
            &proof_json,
            received_at.millis(),
        ],
    )?;
    if inserted == 0 {
        let exact = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM effect_coordination_receipts
                 WHERE attempt_id = ?1 AND target_id = ?2 AND contract_version = ?3
                   AND contract_json = ?4 AND state = ?5 AND proof_version = ?6
                   AND proof_json = ?7
             )",
            params![
                attempt.identity().as_str(),
                receipt.target.as_str(),
                contract_version,
                contract_json,
                state,
                proof_version,
                proof_json,
            ],
            |row| row.get::<_, i64>(0),
        )? != 0;
        if !exact {
            return Err(DatabaseError::CorruptEffectState(
                "conflicting immutable Effect coordination receipt".to_string(),
            ));
        }
    }
    Ok(())
}

/// Reconstructs one immutable operation and its legal progress enum.
pub(super) fn map_operation(row: &rusqlite::Row<'_>) -> Result<EffectOperation, DatabaseError> {
    let prepared_at = LocalTimestamp::from_millis(row.get::<_, i64>("prepared_at")?);
    let phase = row.get::<_, String>("phase")?;
    let progress = match phase.as_str() {
        "prepared" => OperationProgress::Prepared { prepared_at },
        "applied" => OperationProgress::Applied {
            prepared_at,
            applied_at: LocalTimestamp::from_millis(
                row.get::<_, Option<i64>>("applied_at")?.ok_or_else(|| {
                    DatabaseError::CorruptEffectState(
                        "Applied operation lacks timestamp".to_string(),
                    )
                })?,
            ),
        },
        "recovery_required" => OperationProgress::RecoveryRequired {
            prepared_at,
            applied_at: row
                .get::<_, Option<i64>>("applied_at")?
                .map(LocalTimestamp::from_millis),
            detected_at: LocalTimestamp::from_millis(row.get::<_, i64>("updated_at")?),
        },
        "finalized" => OperationProgress::Finalized {
            prepared_at,
            applied_at: LocalTimestamp::from_millis(
                row.get::<_, Option<i64>>("applied_at")?.ok_or_else(|| {
                    DatabaseError::CorruptEffectState(
                        "Finalized operation lacks applied timestamp".to_string(),
                    )
                })?,
            ),
            finalized_at: LocalTimestamp::from_millis(
                row.get::<_, Option<i64>>("finalized_at")?.ok_or_else(|| {
                    DatabaseError::CorruptEffectState(
                        "Finalized operation lacks finalized timestamp".to_string(),
                    )
                })?,
            ),
        },
        other => {
            return Err(DatabaseError::CorruptEffectState(format!(
                "unknown Effect operation phase {other}"
            )));
        }
    };
    EffectOperation::restore(
        EffectOperationId::new(row.get::<_, String>("id")?),
        EffectOperationIntent {
            attempt: ora_effect::ReconcileAttemptId::new(row.get::<_, String>("attempt_id")?),
            resource: EffectResourceId::new(row.get::<_, String>("resource_id")?),
            generation: generation_from_sql(row.get::<_, i64>("generation")?)?,
            sequence: u32::try_from(row.get::<_, i64>("sequence")?).map_err(|_| {
                DatabaseError::CorruptEffectState("invalid operation sequence".to_string())
            })?,
            mutation: parse_mutation(&row.get::<_, String>("mutation")?)?,
            expected: parse_effect_json(row.get::<_, String>("expected_json")?)?,
            planned: parse_effect_json(row.get::<_, String>("planned_json")?)?,
            payload: parse_effect_json(row.get::<_, String>("payload_json")?)?,
        },
        progress,
    )
    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))
}

/// Maps durable attempt phase text into the closed domain phase set.
pub(super) fn parse_attempt_phase(value: &str) -> Result<ReconcileAttemptPhase, DatabaseError> {
    match value {
        "prepared" => Ok(ReconcileAttemptPhase::Prepared),
        "coordinated" => Ok(ReconcileAttemptPhase::Coordinated),
        "applied" => Ok(ReconcileAttemptPhase::Applied),
        "verified" => Ok(ReconcileAttemptPhase::Verified),
        "activated" => Ok(ReconcileAttemptPhase::Activated),
        "finalized" => Ok(ReconcileAttemptPhase::Finalized),
        "recovery_required" => Ok(ReconcileAttemptPhase::RecoveryRequired),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect attempt phase {other}"
        ))),
    }
}

/// Encodes the closed attempt phase set without allowing caller-defined persistence values.
pub(super) fn attempt_phase_value(phase: ReconcileAttemptPhase) -> &'static str {
    match phase {
        ReconcileAttemptPhase::Prepared => "prepared",
        ReconcileAttemptPhase::Coordinated => "coordinated",
        ReconcileAttemptPhase::Applied => "applied",
        ReconcileAttemptPhase::Verified => "verified",
        ReconcileAttemptPhase::Activated => "activated",
        ReconcileAttemptPhase::Finalized => "finalized",
        ReconcileAttemptPhase::RecoveryRequired => "recovery_required",
    }
}

/// Allows idempotent replay or a forward-only durable attempt phase transition.
pub(super) fn attempt_progress_can_advance(
    stored: ReconcileAttemptPhase,
    proposed: ReconcileAttemptPhase,
) -> bool {
    stored == proposed
        || matches!(
            (stored, proposed),
            (
                ReconcileAttemptPhase::Prepared,
                ReconcileAttemptPhase::Coordinated
            ) | (
                ReconcileAttemptPhase::Coordinated,
                ReconcileAttemptPhase::Applied
            ) | (
                ReconcileAttemptPhase::Applied,
                ReconcileAttemptPhase::Verified
            ) | (
                ReconcileAttemptPhase::Verified,
                ReconcileAttemptPhase::Activated
            ) | (
                ReconcileAttemptPhase::Activated,
                ReconcileAttemptPhase::Finalized
            ) | (_, ReconcileAttemptPhase::RecoveryRequired)
        )
}

/// Encodes the closed artifact role set for storage.
fn artifact_role_value(role: ArtifactRole) -> &'static str {
    match role {
        ArtifactRole::Staging => "staging",
        ArtifactRole::Backup => "backup",
        ArtifactRole::Compensation => "compensation",
    }
}

/// Encodes cleanup state without treating finalization as artifact cleanup.
fn artifact_state_value(state: ArtifactState) -> &'static str {
    match state {
        ArtifactState::Reserved => "reserved",
        ArtifactState::Retained => "retained",
        ArtifactState::PendingCleanup => "pending_cleanup",
        ArtifactState::CleanupFailed => "cleanup_failed",
    }
}

/// Allows only idempotent or forward operation progress, with recovery from unfinished phases.
fn operation_progress_can_advance(stored: &str, proposed: &str) -> bool {
    stored == proposed
        || matches!(
            (stored, proposed),
            ("prepared", "applied")
                | ("applied", "finalized")
                | ("prepared" | "applied", "recovery_required")
        )
}
