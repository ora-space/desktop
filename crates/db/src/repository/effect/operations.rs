//! Immutable Operation intent, independent phase evidence, and row audit persistence.
use super::mapping::{
    effect_json, generation_from_sql, generation_to_sql, mutation_value, parse_effect_json,
    parse_mutation,
};
use crate::DatabaseError;
use ora_effect::{
    EffectOperation, EffectOperationId, EffectOperationIntent, EffectResourceId, LocalTimestamp,
    OperationProgress,
};
use rusqlite::{Transaction, params};

/// Inserts one immutable operation intent and its initial Prepared progress.
pub(super) fn insert_operation(
    transaction: &Transaction<'_>,
    operation: &EffectOperation,
    written_at: LocalTimestamp,
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
                   'prepared', ?10, NULL, NULL, ?11)",
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
            written_at.max(*prepared_at).millis(),
        ],
    )?;
    Ok(())
}

/// Updates only legal operation progress columns; immutable intent is protected by a trigger.
pub(super) fn update_operation(
    transaction: &Transaction<'_>,
    operation: &EffectOperation,
    written_at: LocalTimestamp,
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
         SET phase = ?2, applied_at = ?3, finalized_at = ?4, updated_at = MAX(updated_at, ?5), detected_at = ?8
         WHERE id = ?1 AND prepared_at = ?6 AND attempt_id = ?7",
        params![
            operation.identity().as_str(),
            phase,
            applied_at.map(LocalTimestamp::millis),
            finalized_at.map(LocalTimestamp::millis),
            written_at.max(updated_at).millis(),
            prepared_at.millis(),
            operation.attempt().as_str(),
            match operation.progress() {
                OperationProgress::RecoveryRequired { detected_at, .. } => Some(detected_at.millis()),
                OperationProgress::Prepared { .. } | OperationProgress::Applied { .. } | OperationProgress::Finalized { .. } => None,
            },
        ],
    )?;
    if updated != 1 {
        return Err(DatabaseError::CorruptEffectState(
            "Effect operation progress did not match immutable journal input".to_string(),
        ));
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
            detected_at: LocalTimestamp::from_millis(row.get::<_, i64>("detected_at")?),
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
