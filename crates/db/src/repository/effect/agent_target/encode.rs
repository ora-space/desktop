//! Enum and numeric encodings shared by Agent Target SQLite persistence.

use crate::DatabaseError;
use ora_effect::{
    AgentTargetConditionReason, AgentTargetLifecycle, AgentTargetPhase, AgentTargetReconcileState,
    AgentTargetRepositoryError, AgentTargetWakeReason, ConditionImpact, Generation,
};
pub(super) fn map_db_error(error: DatabaseError) -> AgentTargetRepositoryError {
    match error {
        DatabaseError::CorruptEffectState(message) => AgentTargetRepositoryError::corrupt(message),
        other => AgentTargetRepositoryError::storage(other),
    }
}

pub(super) fn generation_to_sql(generation: Generation) -> Result<i64, DatabaseError> {
    i64::try_from(generation.value()).map_err(|_| {
        DatabaseError::CorruptEffectState("generation exceeds SQLite integer range".to_string())
    })
}

pub(super) fn generation_from_sql(value: i64) -> Result<Generation, DatabaseError> {
    u64::try_from(value)
        .map(Generation::new)
        .map_err(|_| DatabaseError::CorruptEffectState("negative generation".to_string()))
}

pub(super) fn generation_from_row(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> Result<Generation, rusqlite::Error> {
    let value: i64 = row.get(index)?;
    generation_from_sql(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

pub(super) fn u64_to_sql(value: u64, field: &str) -> Result<i64, DatabaseError> {
    i64::try_from(value).map_err(|_| {
        DatabaseError::CorruptEffectState(format!("{field} exceeds SQLite integer range"))
    })
}

pub(super) fn u64_from_row(row: &rusqlite::Row<'_>, index: usize) -> Result<u64, rusqlite::Error> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(DatabaseError::CorruptEffectState(
                "negative status version".to_string(),
            )),
        )
    })
}

pub(super) fn u32_from_row(row: &rusqlite::Row<'_>, index: usize) -> Result<u32, rusqlite::Error> {
    let value: i64 = row.get(index)?;
    u32::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(DatabaseError::CorruptEffectState(
                "invalid attempt count".to_string(),
            )),
        )
    })
}

pub(super) fn lifecycle_value(lifecycle: AgentTargetLifecycle) -> &'static str {
    match lifecycle {
        AgentTargetLifecycle::Active => "active",
        AgentTargetLifecycle::Retired => "retired",
    }
}

pub(super) fn parse_lifecycle(value: &str) -> Result<AgentTargetLifecycle, DatabaseError> {
    match value {
        "active" => Ok(AgentTargetLifecycle::Active),
        "retired" => Ok(AgentTargetLifecycle::Retired),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown agent target lifecycle".to_string(),
        )),
    }
}

pub(super) fn phase_value(phase: AgentTargetPhase) -> &'static str {
    match phase {
        AgentTargetPhase::Pending => "pending",
        AgentTargetPhase::WaitingForIdle => "waiting_for_idle",
        AgentTargetPhase::Quiescing => "quiescing",
        AgentTargetPhase::Applying => "applying",
        AgentTargetPhase::Resuming => "resuming",
        AgentTargetPhase::Current => "current",
        AgentTargetPhase::ReadyWithIssues => "ready_with_issues",
        AgentTargetPhase::Degraded => "degraded",
        AgentTargetPhase::Retiring => "retiring",
        AgentTargetPhase::RecoveryRequired => "recovery_required",
    }
}

pub(super) fn parse_phase(value: &str) -> Result<AgentTargetPhase, DatabaseError> {
    match value {
        "pending" => Ok(AgentTargetPhase::Pending),
        "waiting_for_idle" => Ok(AgentTargetPhase::WaitingForIdle),
        "quiescing" => Ok(AgentTargetPhase::Quiescing),
        "applying" => Ok(AgentTargetPhase::Applying),
        "resuming" => Ok(AgentTargetPhase::Resuming),
        "current" => Ok(AgentTargetPhase::Current),
        "ready_with_issues" => Ok(AgentTargetPhase::ReadyWithIssues),
        "degraded" => Ok(AgentTargetPhase::Degraded),
        "retiring" => Ok(AgentTargetPhase::Retiring),
        "recovery_required" => Ok(AgentTargetPhase::RecoveryRequired),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown agent target phase".to_string(),
        )),
    }
}

pub(super) fn impact_value(impact: ConditionImpact) -> &'static str {
    match impact {
        ConditionImpact::Blocking => "blocking",
        ConditionImpact::NonBlocking => "non_blocking",
    }
}

pub(super) fn parse_impact(value: &str) -> Result<ConditionImpact, DatabaseError> {
    match value {
        "blocking" => Ok(ConditionImpact::Blocking),
        "non_blocking" => Ok(ConditionImpact::NonBlocking),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown condition impact".to_string(),
        )),
    }
}

pub(super) fn reconcile_state_value(state: AgentTargetReconcileState) -> &'static str {
    match state {
        AgentTargetReconcileState::Pending => "pending",
        AgentTargetReconcileState::Claimed => "claimed",
        AgentTargetReconcileState::Blocked => "blocked",
        AgentTargetReconcileState::RetryScheduled => "retry_scheduled",
    }
}

pub(super) fn parse_reconcile_state(
    value: &str,
) -> Result<AgentTargetReconcileState, DatabaseError> {
    match value {
        "pending" => Ok(AgentTargetReconcileState::Pending),
        "claimed" => Ok(AgentTargetReconcileState::Claimed),
        "blocked" => Ok(AgentTargetReconcileState::Blocked),
        "retry_scheduled" => Ok(AgentTargetReconcileState::RetryScheduled),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown agent target reconcile state".to_string(),
        )),
    }
}

pub(super) fn wake_reason_value(reason: AgentTargetWakeReason) -> &'static str {
    match reason {
        AgentTargetWakeReason::DesiredChanged => "desired_changed",
        AgentTargetWakeReason::CapabilityChanged => "capability_changed",
        AgentTargetWakeReason::Retry => "retry",
        AgentTargetWakeReason::Recovery => "recovery",
        AgentTargetWakeReason::StartupRepair => "startup_repair",
    }
}

pub(super) fn parse_wake_reason(value: &str) -> Result<AgentTargetWakeReason, DatabaseError> {
    match value {
        "desired_changed" => Ok(AgentTargetWakeReason::DesiredChanged),
        "capability_changed" => Ok(AgentTargetWakeReason::CapabilityChanged),
        "retry" => Ok(AgentTargetWakeReason::Retry),
        "recovery" => Ok(AgentTargetWakeReason::Recovery),
        "startup_repair" => Ok(AgentTargetWakeReason::StartupRepair),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown agent target wake reason".to_string(),
        )),
    }
}

pub(super) fn condition_reason_value(reason: AgentTargetConditionReason) -> &'static str {
    match reason {
        AgentTargetConditionReason::NoConsumers => "no_consumers",
        AgentTargetConditionReason::IncompatibleSurfaceDeclarations => {
            "incompatible_surface_declarations"
        }
        AgentTargetConditionReason::DesiredCollision => "desired_collision",
        AgentTargetConditionReason::PreservedConflict => "preserved_conflict",
        AgentTargetConditionReason::OwnershipConflict => "ownership_conflict",
        AgentTargetConditionReason::DriftConflict => "drift_conflict",
        AgentTargetConditionReason::SourceUnavailable => "source_unavailable",
        AgentTargetConditionReason::PathUnsafe => "path_unsafe",
        AgentTargetConditionReason::ScanFailed => "scan_failed",
        AgentTargetConditionReason::WaitingForIdle => "waiting_for_idle",
        AgentTargetConditionReason::ConsumerResumeFailed => "consumer_resume_failed",
        AgentTargetConditionReason::MaterializationFailed => "materialization_failed",
        AgentTargetConditionReason::TransientIo => "transient_io",
        AgentTargetConditionReason::RecoveryRequired => "recovery_required",
        AgentTargetConditionReason::UnsupportedByAgent => "unsupported_by_agent",
        AgentTargetConditionReason::CapabilityInvalid => "capability_invalid",
    }
}

pub(super) fn parse_condition_reason(
    value: &str,
) -> Result<AgentTargetConditionReason, DatabaseError> {
    match value {
        "no_consumers" => Ok(AgentTargetConditionReason::NoConsumers),
        "incompatible_surface_declarations" => {
            Ok(AgentTargetConditionReason::IncompatibleSurfaceDeclarations)
        }
        "desired_collision" => Ok(AgentTargetConditionReason::DesiredCollision),
        "preserved_conflict" => Ok(AgentTargetConditionReason::PreservedConflict),
        "ownership_conflict" => Ok(AgentTargetConditionReason::OwnershipConflict),
        "drift_conflict" => Ok(AgentTargetConditionReason::DriftConflict),
        "source_unavailable" => Ok(AgentTargetConditionReason::SourceUnavailable),
        "path_unsafe" => Ok(AgentTargetConditionReason::PathUnsafe),
        "scan_failed" => Ok(AgentTargetConditionReason::ScanFailed),
        "waiting_for_idle" => Ok(AgentTargetConditionReason::WaitingForIdle),
        "consumer_resume_failed" => Ok(AgentTargetConditionReason::ConsumerResumeFailed),
        "materialization_failed" => Ok(AgentTargetConditionReason::MaterializationFailed),
        "transient_io" => Ok(AgentTargetConditionReason::TransientIo),
        "recovery_required" => Ok(AgentTargetConditionReason::RecoveryRequired),
        "unsupported_by_agent" => Ok(AgentTargetConditionReason::UnsupportedByAgent),
        "capability_invalid" => Ok(AgentTargetConditionReason::CapabilityInvalid),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown agent target condition reason".to_string(),
        )),
    }
}
