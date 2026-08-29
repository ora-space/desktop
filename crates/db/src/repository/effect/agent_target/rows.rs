//! Row mapping for Agent Target identity, status, and reconcile requests.

use super::conditions::load_target_conditions;
use super::encode::*;
use crate::DatabaseError;
use ora_domain::WorkspaceId;
use ora_effect::{
    AgentCapabilityRevision, AgentPluginId, AgentTarget, AgentTargetId, AgentTargetIdentity,
    AgentTargetReconcileRequest, AgentTargetReconcileState, AgentTargetRepositoryError,
    AgentTargetStatus, AgentTargetWakeReason, Generation,
};
use rusqlite::params;

/// Reconstructs an Agent Target identity row without scheduling state.
pub(super) fn map_agent_target_row(
    row: &rusqlite::Row<'_>,
) -> Result<AgentTarget, rusqlite::Error> {
    Ok(AgentTarget {
        id: AgentTargetId::new(row.get::<_, String>(0)?),
        identity: AgentTargetIdentity::new(
            WorkspaceId::new(row.get::<_, String>(1)?),
            AgentPluginId::new(row.get::<_, String>(2)?),
        ),
        capability_revision: AgentCapabilityRevision::new(row.get::<_, String>(3)?),
        lifecycle: parse_lifecycle(&row.get::<_, String>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub(super) struct RequestColumns {
    pub requested_generation: Generation,
    pub request_token: String,
    pub state: AgentTargetReconcileState,
    pub attempt_count: u32,
    pub requested_at: i64,
    pub not_before_at: i64,
    pub updated_at: i64,
}

/// Validates wake_reason even though coalescing replaces it with the newest wake.
pub(super) fn map_request_columns(
    row: &rusqlite::Row<'_>,
) -> Result<RequestColumns, rusqlite::Error> {
    let _: AgentTargetWakeReason =
        parse_wake_reason(&row.get::<_, String>(3)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(RequestColumns {
        requested_generation: generation_from_row(row, 0)?,
        request_token: row.get(1)?,
        state: parse_reconcile_state(
            &row.get::<_, String>(2)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
        )
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        attempt_count: u32_from_row(row, 7)?,
        requested_at: row.get(8)?,
        not_before_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

/// Reconstructs a complete request including the target identity join columns.
pub(super) fn map_full_request_row(
    row: &rusqlite::Row<'_>,
) -> Result<AgentTargetReconcileRequest, rusqlite::Error> {
    Ok(AgentTargetReconcileRequest {
        agent_target_id: AgentTargetId::new(row.get::<_, String>(0)?),
        identity: AgentTargetIdentity::new(
            WorkspaceId::new(row.get::<_, String>(1)?),
            AgentPluginId::new(row.get::<_, String>(2)?),
        ),
        requested_generation: generation_from_row(row, 3)?,
        request_token: row.get(4)?,
        state: parse_reconcile_state(
            &row.get::<_, String>(5)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
        )
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        wake_reason: parse_wake_reason(&row.get::<_, String>(6)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        attempt_count: u32_from_row(row, 10)?,
        requested_at: row.get(11)?,
        not_before_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

/// Loads status then attaches owned conditions so callers never observe a half-read target.
pub(super) fn load_status_for_target(
    connection: &rusqlite::Connection,
    target: &AgentTarget,
) -> Result<AgentTargetStatus, DatabaseError> {
    let mut status = connection.query_row(
        "SELECT desired_generation, observed_generation, applied_generation, ready_generation,
                phase, status_version, created_at, updated_at
         FROM effect_agent_target_status
         WHERE agent_target_id = ?1",
        params![target.id.as_str()],
        |row| {
            Ok(AgentTargetStatus {
                agent_target_id: target.id.clone(),
                identity: target.identity.clone(),
                desired_generation: generation_from_row(row, 0)?,
                observed_generation: generation_from_row(row, 1)?,
                applied_generation: generation_from_row(row, 2)?,
                ready_generation: generation_from_row(row, 3)?,
                phase: parse_phase(&row.get::<_, String>(4)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                status_version: u64_from_row(row, 5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                conditions: Vec::new(),
            })
        },
    )?;
    status.conditions = load_target_conditions(connection, target.id.as_str())?;
    Ok(status)
}

/// Enforces generation order in the domain so a CHECK-less write path cannot persist an illegal tuple.
pub(super) fn validate_generation_order(
    status: &AgentTargetStatus,
) -> Result<(), AgentTargetRepositoryError> {
    if status.applied_generation > status.observed_generation
        || status.observed_generation > status.desired_generation
        || status.ready_generation > status.applied_generation
    {
        return Err(AgentTargetRepositoryError::corrupt(
            "agent target generation ordering is illegal",
        ));
    }
    Ok(())
}
