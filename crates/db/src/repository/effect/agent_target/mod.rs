//! Typed SQLite persistence for Agent Target Expand-phase Effect state.
//!
//! These APIs coexist with the surface-keyed Skill repository. Production workers must not claim
//! target reconcile requests until a later Contract ticket switches ownership.

mod encode;

use super::SqliteEffectRepository;
use crate::DatabaseError;
use encode::*;
use ora_domain::WorkspaceId;
use ora_effect::{
    AgentCapabilityRevision, AgentPluginId, AgentTarget, AgentTargetCondition,
    AgentTargetConditionSubject, AgentTargetId, AgentTargetIdentity, AgentTargetLifecycle,
    AgentTargetReconcileRequest, AgentTargetReconcileState, AgentTargetRecord,
    AgentTargetRepository, AgentTargetRepositoryError, AgentTargetStatus, AgentTargetWakeReason,
    ConsumerId, Generation, SurfaceKey,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

impl AgentTargetRepository for SqliteEffectRepository {
    fn upsert_agent_target(
        &self,
        identity: &AgentTargetIdentity,
        capability_revision: &AgentCapabilityRevision,
        lifecycle: AgentTargetLifecycle,
        updated_at: i64,
    ) -> Result<AgentTarget, AgentTargetRepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                ensure_workspace_effect(&transaction, &identity.workspace_id, updated_at)?;
                let existing = transaction
                    .query_row(
                        "SELECT id, created_at FROM effect_agent_targets
                         WHERE workspace_id = ?1 AND agent_plugin_id = ?2",
                        params![
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str()
                        ],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?;
                let (id, created_at) = if let Some((id, created_at)) = existing {
                    transaction.execute(
                        "UPDATE effect_agent_targets
                         SET capability_revision = ?1, lifecycle = ?2, updated_at = ?3
                         WHERE id = ?4",
                        params![
                            capability_revision.as_str(),
                            lifecycle_value(lifecycle),
                            updated_at,
                            &id
                        ],
                    )?;
                    (id, created_at)
                } else {
                    let id = Uuid::new_v4().to_string();
                    transaction.execute(
                        "INSERT INTO effect_agent_targets (
                             id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                             created_at, updated_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                        params![
                            &id,
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str(),
                            capability_revision.as_str(),
                            lifecycle_value(lifecycle),
                            updated_at,
                        ],
                    )?;
                    transaction.execute(
                        "INSERT INTO effect_agent_target_status (
                             agent_target_id, desired_generation, observed_generation,
                             applied_generation, ready_generation, phase, status_version,
                             created_at, updated_at
                         ) VALUES (?1, 0, 0, 0, 0, 'current', 1, ?2, ?2)",
                        params![&id, updated_at],
                    )?;
                    (id, updated_at)
                };
                transaction.commit()?;
                Ok(AgentTarget {
                    id: AgentTargetId::new(id),
                    identity: identity.clone(),
                    capability_revision: capability_revision.clone(),
                    lifecycle,
                    created_at,
                    updated_at,
                })
            })
            .map_err(map_db_error)
    }

    fn load_agent_target(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTarget>, AgentTargetRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                                created_at, updated_at
                         FROM effect_agent_targets
                         WHERE workspace_id = ?1 AND agent_plugin_id = ?2",
                        params![
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str()
                        ],
                        map_agent_target_row,
                    )
                    .optional()
                    .map_err(Into::into)
            })
            .map_err(map_db_error)
    }

    fn load_agent_target_by_id(
        &self,
        agent_target_id: &AgentTargetId,
    ) -> Result<Option<AgentTarget>, AgentTargetRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                                created_at, updated_at
                         FROM effect_agent_targets
                         WHERE id = ?1",
                        params![agent_target_id.as_str()],
                        map_agent_target_row,
                    )
                    .optional()
                    .map_err(Into::into)
            })
            .map_err(map_db_error)
    }

    fn save_agent_target_status(
        &self,
        status: &AgentTargetStatus,
    ) -> Result<(), AgentTargetRepositoryError> {
        validate_generation_order(status)?;
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let updated = transaction.execute(
                    "UPDATE effect_agent_target_status
                     SET desired_generation = ?1, observed_generation = ?2,
                         applied_generation = ?3, ready_generation = ?4, phase = ?5,
                         status_version = ?6, updated_at = ?7
                     WHERE agent_target_id = ?8",
                    params![
                        generation_to_sql(status.desired_generation)?,
                        generation_to_sql(status.observed_generation)?,
                        generation_to_sql(status.applied_generation)?,
                        generation_to_sql(status.ready_generation)?,
                        phase_value(status.phase),
                        u64_to_sql(status.status_version, "status_version")?,
                        status.updated_at,
                        status.agent_target_id.as_str(),
                    ],
                )?;
                if updated == 0 {
                    return Err(DatabaseError::CorruptEffectState(
                        "agent target status row is missing".to_string(),
                    ));
                }
                replace_target_conditions(
                    &transaction,
                    status.agent_target_id.as_str(),
                    &status.conditions,
                )?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(map_db_error)
    }

    fn load_agent_target_status(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTargetStatus>, AgentTargetRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let Some(target) = connection
                    .query_row(
                        "SELECT id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                                created_at, updated_at
                         FROM effect_agent_targets
                         WHERE workspace_id = ?1 AND agent_plugin_id = ?2",
                        params![
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str()
                        ],
                        map_agent_target_row,
                    )
                    .optional()?
                else {
                    return Ok(None);
                };
                load_status_for_target(connection, &target).map(Some)
            })
            .map_err(map_db_error)
    }

    fn upsert_agent_target_reconcile_request(
        &self,
        identity: &AgentTargetIdentity,
        requested_generation: Generation,
        wake_reason: AgentTargetWakeReason,
        not_before_at: i64,
        updated_at: i64,
    ) -> Result<AgentTargetReconcileRequest, AgentTargetRepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let target_id: String = transaction
                    .query_row(
                        "SELECT id FROM effect_agent_targets
                         WHERE workspace_id = ?1 AND agent_plugin_id = ?2",
                        params![
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str()
                        ],
                        |row| row.get(0),
                    )
                    .map_err(|_| {
                        DatabaseError::CorruptEffectState(format!(
                            "agent target missing for workspace {} and plugin {}",
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str()
                        ))
                    })?;
                let existing = transaction
                    .query_row(
                        "SELECT requested_generation, request_token, state, wake_reason,
                                blocked_reason, lease_owner, lease_expires_at, attempt_count,
                                requested_at, not_before_at, updated_at
                         FROM effect_agent_target_reconcile_requests
                         WHERE agent_target_id = ?1",
                        params![&target_id],
                        map_request_columns,
                    )
                    .optional()?;
                let request = if let Some(existing) = existing {
                    // Keep the highest requested generation and the earliest due time across wakes.
                    let requested_generation =
                        existing.requested_generation.max(requested_generation);
                    let wake_requested_at = updated_at.min(not_before_at);
                    let requested_at = existing.requested_at.min(wake_requested_at);
                    let due_at = existing.not_before_at.min(not_before_at).max(requested_at);
                    let (state, blocked_reason, lease_owner, lease_expires_at) =
                        if existing.state == AgentTargetReconcileState::Claimed {
                            (
                                existing.state,
                                existing.blocked_reason,
                                existing.lease_owner,
                                existing.lease_expires_at,
                            )
                        } else {
                            (AgentTargetReconcileState::Pending, None, None, None)
                        };
                    let updated_at = updated_at.max(existing.updated_at);
                    transaction.execute(
                        "UPDATE effect_agent_target_reconcile_requests
                         SET requested_generation = ?1, state = ?2, wake_reason = ?3,
                             blocked_reason = ?4, lease_owner = ?5, lease_expires_at = ?6,
                             requested_at = ?7, not_before_at = ?8, updated_at = ?9
                         WHERE agent_target_id = ?10",
                        params![
                            generation_to_sql(requested_generation)?,
                            reconcile_state_value(state),
                            wake_reason_value(wake_reason),
                            blocked_reason.as_deref(),
                            lease_owner.as_deref(),
                            lease_expires_at,
                            requested_at,
                            due_at,
                            updated_at,
                            &target_id,
                        ],
                    )?;
                    AgentTargetReconcileRequest {
                        agent_target_id: AgentTargetId::new(target_id),
                        identity: identity.clone(),
                        requested_generation,
                        request_token: existing.request_token,
                        state,
                        wake_reason,
                        blocked_reason,
                        lease_owner,
                        lease_expires_at,
                        attempt_count: existing.attempt_count,
                        requested_at,
                        not_before_at: due_at,
                        updated_at,
                    }
                } else {
                    let token = Uuid::new_v4().to_string();
                    let requested_at = updated_at.min(not_before_at);
                    let due_at = not_before_at.max(requested_at);
                    transaction.execute(
                        "INSERT INTO effect_agent_target_reconcile_requests (
                             agent_target_id, requested_generation, request_token, state,
                             wake_reason, blocked_reason, lease_owner, lease_expires_at,
                             attempt_count, requested_at, not_before_at, updated_at
                         ) VALUES (?1, ?2, ?3, 'pending', ?4, NULL, NULL, NULL, 0, ?5, ?6, ?7)",
                        params![
                            &target_id,
                            generation_to_sql(requested_generation)?,
                            &token,
                            wake_reason_value(wake_reason),
                            requested_at,
                            due_at,
                            updated_at,
                        ],
                    )?;
                    AgentTargetReconcileRequest {
                        agent_target_id: AgentTargetId::new(target_id),
                        identity: identity.clone(),
                        requested_generation,
                        request_token: token,
                        state: AgentTargetReconcileState::Pending,
                        wake_reason,
                        blocked_reason: None,
                        lease_owner: None,
                        lease_expires_at: None,
                        attempt_count: 0,
                        requested_at,
                        not_before_at: due_at,
                        updated_at,
                    }
                };
                transaction.commit()?;
                Ok(request)
            })
            .map_err(map_db_error)
    }

    fn load_agent_target_reconcile_request(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTargetReconcileRequest>, AgentTargetRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT targets.id, targets.workspace_id, targets.agent_plugin_id,
                                requests.requested_generation, requests.request_token,
                                requests.state, requests.wake_reason, requests.blocked_reason,
                                requests.lease_owner, requests.lease_expires_at,
                                requests.attempt_count, requests.requested_at,
                                requests.not_before_at, requests.updated_at
                         FROM effect_agent_target_reconcile_requests requests
                         JOIN effect_agent_targets targets
                           ON targets.id = requests.agent_target_id
                         WHERE targets.workspace_id = ?1 AND targets.agent_plugin_id = ?2",
                        params![
                            identity.workspace_id.as_ref(),
                            identity.agent_plugin_id.as_str()
                        ],
                        map_full_request_row,
                    )
                    .optional()
                    .map_err(Into::into)
            })
            .map_err(map_db_error)
    }

    fn replace_agent_target_conditions(
        &self,
        agent_target_id: &AgentTargetId,
        conditions: &[AgentTargetCondition],
    ) -> Result<(), AgentTargetRepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                replace_target_conditions(&transaction, agent_target_id.as_str(), conditions)?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(map_db_error)
    }

    fn load_agent_target_record(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTargetRecord>, AgentTargetRepositoryError> {
        let Some(target) = self.load_agent_target(identity)? else {
            return Ok(None);
        };
        let status = self.load_agent_target_status(identity)?.ok_or_else(|| {
            AgentTargetRepositoryError::corrupt("agent target status row is missing")
        })?;
        let reconcile_request = self.load_agent_target_reconcile_request(identity)?;
        Ok(Some(AgentTargetRecord {
            target,
            status,
            reconcile_request,
        }))
    }

    fn list_agent_targets_for_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<AgentTarget>, AgentTargetRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                            created_at, updated_at
                     FROM effect_agent_targets
                     WHERE workspace_id = ?1
                     ORDER BY agent_plugin_id",
                )?;
                let rows =
                    statement.query_map(params![workspace_id.as_ref()], map_agent_target_row)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            })
            .map_err(map_db_error)
    }
}

/// Refuses to create an Agent Target without the Workspace Effect aggregate that owns generations.
fn ensure_workspace_effect(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &WorkspaceId,
    _now: i64,
) -> Result<(), DatabaseError> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM workspace_effects WHERE workspace_id = ?1)",
        params![workspace_id.as_ref()],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    Err(DatabaseError::CorruptEffectState(format!(
        "workspace effect missing for {}",
        workspace_id.as_ref()
    )))
}

fn load_status_for_target(
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

fn load_target_conditions(
    connection: &rusqlite::Connection,
    agent_target_id: &str,
) -> Result<Vec<AgentTargetCondition>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, surface_id, consumer_id, subject_kind, subject_id, reason, impact,
                failed_generation, message, first_observed_at, last_observed_at
         FROM effect_agent_target_conditions
         WHERE agent_target_id = ?1
         ORDER BY subject_kind, subject_id, reason, IFNULL(surface_id, ''), IFNULL(consumer_id, '')",
    )?;
    let rows = statement.query_map(params![agent_target_id], |row| {
        let subject_json: String = row.get(4)?;
        let subject = serde_json::from_str::<AgentTargetConditionSubject>(&subject_json).map_err(
            |error| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            },
        )?;
        let reason_raw: String = row.get(5)?;
        let reason = parse_condition_reason(&reason_raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let impact = parse_impact(&row.get::<_, String>(6)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let failed_generation = match row.get::<_, Option<i64>>(7)? {
            Some(value) => Some(generation_from_sql(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })?),
            None => None,
        };
        let surface_id: Option<String> = row.get(1)?;
        let consumer_id: Option<String> = row.get(2)?;
        Ok(AgentTargetCondition {
            id: row.get(0)?,
            subject,
            reason,
            impact,
            message: row.get(8)?,
            first_observed_at: row.get(9)?,
            last_observed_at: row.get(10)?,
            failed_generation,
            surface_key: surface_id.map(SurfaceKey::new),
            consumer_id: consumer_id.map(ConsumerId::new),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn replace_target_conditions(
    transaction: &rusqlite::Transaction<'_>,
    agent_target_id: &str,
    conditions: &[AgentTargetCondition],
) -> Result<(), DatabaseError> {
    transaction.execute(
        "DELETE FROM effect_agent_target_conditions WHERE agent_target_id = ?1",
        params![agent_target_id],
    )?;
    for condition in conditions {
        let subject_kind = match &condition.subject {
            AgentTargetConditionSubject::AgentTarget => "agent_target",
            AgentTargetConditionSubject::Surface { .. } => "surface",
            AgentTargetConditionSubject::Consumer { .. } => "consumer",
            AgentTargetConditionSubject::DesiredSkill { .. } => "desired_item",
            AgentTargetConditionSubject::ManagedSkill { .. } => "managed_item",
            AgentTargetConditionSubject::Mcp { .. } => "mcp",
        };
        let subject_id = serde_json::to_string(&condition.subject)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        let id = if condition.id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            condition.id.clone()
        };
        transaction.execute(
            "INSERT INTO effect_agent_target_conditions (
                 id, agent_target_id, surface_id, consumer_id, subject_kind, subject_id, reason,
                 impact, failed_generation, message, first_observed_at, last_observed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                agent_target_id,
                condition.surface_key.as_ref().map(SurfaceKey::as_str),
                condition.consumer_id.as_ref().map(ConsumerId::as_str),
                subject_kind,
                subject_id,
                condition_reason_value(condition.reason),
                impact_value(condition.impact),
                condition
                    .failed_generation
                    .map(generation_to_sql)
                    .transpose()?,
                &condition.message,
                condition.first_observed_at,
                condition.last_observed_at,
            ],
        )?;
    }
    Ok(())
}

fn map_agent_target_row(row: &rusqlite::Row<'_>) -> Result<AgentTarget, rusqlite::Error> {
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

struct RequestColumns {
    requested_generation: Generation,
    request_token: String,
    state: AgentTargetReconcileState,
    blocked_reason: Option<String>,
    lease_owner: Option<String>,
    lease_expires_at: Option<i64>,
    attempt_count: u32,
    requested_at: i64,
    not_before_at: i64,
    updated_at: i64,
}

fn map_request_columns(row: &rusqlite::Row<'_>) -> Result<RequestColumns, rusqlite::Error> {
    // Validate wake_reason even though coalescing replaces it with the newest wake.
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
        state: parse_reconcile_state(&row.get::<_, String>(2)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        blocked_reason: row.get(4)?,
        lease_owner: row.get(5)?,
        lease_expires_at: row.get(6)?,
        attempt_count: u32_from_row(row, 7)?,
        requested_at: row.get(8)?,
        not_before_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn map_full_request_row(
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
        state: parse_reconcile_state(&row.get::<_, String>(5)?).map_err(|error| {
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
        blocked_reason: row.get(7)?,
        lease_owner: row.get(8)?,
        lease_expires_at: row.get(9)?,
        attempt_count: u32_from_row(row, 10)?,
        requested_at: row.get(11)?,
        not_before_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn validate_generation_order(status: &AgentTargetStatus) -> Result<(), AgentTargetRepositoryError> {
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
