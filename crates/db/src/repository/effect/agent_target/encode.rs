//! Enum encodings shared by Agent Target SQLite persistence.
//!
//! Generation and unsigned integer conversions live in `mapping` so Skill and Agent Target rows
//! cannot drift onto different overflow rules.

use crate::DatabaseError;
use ora_effect::{
    AgentTargetConditionReason, AgentTargetLifecycle, AgentTargetPhase, AgentTargetReconcileState,
    AgentTargetRepositoryError, AgentTargetWakeReason, ConditionImpact, Generation,
};

pub(super) use super::super::mapping::{generation_from_sql, generation_to_sql, u64_to_sql};

/// Maps repository-facing failures without leaking rusqlite types across the Effect port.
pub(super) fn map_db_error(error: DatabaseError) -> AgentTargetRepositoryError {
    match error {
        DatabaseError::CorruptEffectState(message) => AgentTargetRepositoryError::corrupt(message),
        other => AgentTargetRepositoryError::storage(other),
    }
}

/// Reads a generation by ordinal so Agent Target SELECT lists can stay positional.
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

/// Reads an unsigned counter by ordinal; SQLite cannot store the domain's unsigned type.
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

/// Reads an attempt counter by ordinal; negative values cannot be a legal retry count.
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

/// Writes the CHECK-accepted lifecycle token so domain and schema cannot drift.
pub(super) fn lifecycle_value(lifecycle: AgentTargetLifecycle) -> &'static str {
    match lifecycle {
        AgentTargetLifecycle::Active => "active",
        AgentTargetLifecycle::Retired => "retired",
    }
}

/// Rejects lifecycle strings the CHECK constraint should already have excluded.
pub(super) fn parse_lifecycle(value: &str) -> Result<AgentTargetLifecycle, DatabaseError> {
    match value {
        "active" => Ok(AgentTargetLifecycle::Active),
        "retired" => Ok(AgentTargetLifecycle::Retired),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown agent target lifecycle".to_string(),
        )),
    }
}

/// Writes the CHECK-accepted phase token so domain and schema cannot drift.
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

/// Rejects phase strings the CHECK constraint should already have excluded.
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

/// Writes the CHECK-accepted impact token so domain and schema cannot drift.
pub(super) fn impact_value(impact: ConditionImpact) -> &'static str {
    match impact {
        ConditionImpact::Blocking => "blocking",
        ConditionImpact::NonBlocking => "non_blocking",
    }
}

/// Rejects impact strings the CHECK constraint should already have excluded.
pub(super) fn parse_impact(value: &str) -> Result<ConditionImpact, DatabaseError> {
    match value {
        "blocking" => Ok(ConditionImpact::Blocking),
        "non_blocking" => Ok(ConditionImpact::NonBlocking),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown condition impact".to_string(),
        )),
    }
}

/// Splits the state machine into the four columns SQLite CHECKs as functions of `state`.
pub(super) fn reconcile_state_sql(
    state: &AgentTargetReconcileState,
) -> (&'static str, Option<&str>, Option<&str>, Option<i64>) {
    match state {
        AgentTargetReconcileState::Pending => ("pending", None, None, None),
        AgentTargetReconcileState::RetryScheduled => ("retry_scheduled", None, None, None),
        AgentTargetReconcileState::Claimed {
            lease_owner,
            lease_expires_at,
        } => (
            "claimed",
            None,
            Some(lease_owner.as_str()),
            Some(*lease_expires_at),
        ),
        AgentTargetReconcileState::Blocked { reason } => {
            ("blocked", Some(reason.as_str()), None, None)
        }
    }
}

/// Rebuilds the domain state machine so illegal column combinations cannot enter memory.
pub(super) fn parse_reconcile_state(
    value: &str,
    blocked_reason: Option<String>,
    lease_owner: Option<String>,
    lease_expires_at: Option<i64>,
) -> Result<AgentTargetReconcileState, DatabaseError> {
    match (value, blocked_reason, lease_owner, lease_expires_at) {
        ("pending", None, None, None) => Ok(AgentTargetReconcileState::Pending),
        ("retry_scheduled", None, None, None) => Ok(AgentTargetReconcileState::RetryScheduled),
        ("claimed", None, Some(lease_owner), Some(lease_expires_at)) => {
            Ok(AgentTargetReconcileState::Claimed {
                lease_owner,
                lease_expires_at,
            })
        }
        ("blocked", Some(reason), None, None) => Ok(AgentTargetReconcileState::Blocked { reason }),
        _ => Err(DatabaseError::CorruptEffectState(
            "agent target reconcile state columns are inconsistent".to_string(),
        )),
    }
}

/// Writes the CHECK-accepted wake-reason token so domain and schema cannot drift.
pub(super) fn wake_reason_value(reason: AgentTargetWakeReason) -> &'static str {
    match reason {
        AgentTargetWakeReason::DesiredChanged => "desired_changed",
        AgentTargetWakeReason::CapabilityChanged => "capability_changed",
        AgentTargetWakeReason::Retry => "retry",
        AgentTargetWakeReason::Recovery => "recovery",
        AgentTargetWakeReason::StartupRepair => "startup_repair",
    }
}

/// Rejects wake-reason strings the CHECK constraint should already have excluded.
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

/// Writes the CHECK-accepted condition-reason token so domain and schema cannot drift.
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

/// Rejects condition-reason strings the CHECK constraint should already have excluded.
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
