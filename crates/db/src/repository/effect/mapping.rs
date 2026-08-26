use crate::DatabaseError;
use ora_domain::{Namespace, WorkspaceId};
use ora_effect::{
    AppliedFingerprint, Condition, ConditionReason, ConditionSubject, DesiredSkillState, Digest,
    EffectOperation, EffectOperationPhase, Generation, ManagedIdentity, ManagedSkill,
    OperationState, RepositoryError, SkillName, SkillSelectionKey, SkillSource, SkillState,
    SourceError, SourceKind, SourceVersion, SurfacePhase, SurfaceStatus, WorkspaceEffect,
    WorkspaceEffectSpec,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Loads the complete normalized desired set while treating a missing row as generation zero.
pub(super) fn load_workspace_effect(
    connection: &Connection,
    workspace_id: &WorkspaceId,
) -> Result<WorkspaceEffect, DatabaseError> {
    let generation = current_generation(connection, workspace_id)?;
    let mut statement = connection.prepare(
        "SELECT sources.source_kind, sources.namespace, sources.identifier AS skill_name,
                json_extract(revisions.payload_json, '$.display_name') AS display_name,
                revisions.revision AS source_version, revisions.state_digest AS skill_md_digest
         FROM workspace_effect_desired_items desired
         JOIN effect_sources sources ON sources.id = desired.source_id
         JOIN effect_source_revisions revisions ON revisions.id = desired.revision_id
         WHERE desired.workspace_id = ?1
         ORDER BY sources.source_kind, sources.namespace, sources.identifier",
    )?;
    let mut rows = statement.query(params![workspace_id.as_ref()])?;
    let mut skills = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let (key, state) = map_desired(row)?;
        skills.insert(key, state);
    }
    Ok(WorkspaceEffect {
        workspace_id: workspace_id.clone(),
        generation,
        spec: WorkspaceEffectSpec { skills },
    })
}

/// Reads a generation with checked integer conversion so corrupt negative rows cannot enter state.
pub(super) fn current_generation(
    connection: &Connection,
    workspace_id: &WorkspaceId,
) -> Result<Generation, DatabaseError> {
    let value = connection
        .query_row(
            "SELECT generation FROM workspace_effects WHERE workspace_id = ?1",
            params![workspace_id.as_ref()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    generation_from_sql(value)
}

/// Validates and reconstructs one normalized desired source row.
pub(super) fn map_desired(
    row: &Row<'_>,
) -> Result<(SkillSelectionKey, DesiredSkillState), DatabaseError> {
    let key = map_selection_key(row)?;
    let display_name = SkillName::parse(row.get::<_, String>("display_name")?)
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
    if display_name != key.name {
        return Err(DatabaseError::CorruptEffectState(
            "desired display name has a different identity".to_string(),
        ));
    }
    let version = SourceVersion::parse(row.get::<_, String>("source_version")?)
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
    let source = match key.source_kind {
        SourceKind::Local => SkillSource::Local {
            namespace: key.namespace.clone(),
            version,
        },
        SourceKind::Plugin => SkillSource::Plugin {
            namespace: key.namespace.clone(),
            version,
        },
    };
    let desired = DesiredSkillState::try_new(SkillState {
        name: display_name,
        skill_md_digest: Digest::parse(row.get::<_, String>("skill_md_digest")?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        source,
    })
    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
    Ok((key, desired))
}

/// Reconstructs a case-insensitive selection identity from common source columns.
pub(super) fn map_selection_key(row: &Row<'_>) -> Result<SkillSelectionKey, DatabaseError> {
    Ok(SkillSelectionKey::new(
        parse_source_kind(&row.get::<_, String>("source_kind")?)?,
        Namespace::new(row.get::<_, String>("namespace")?)?,
        SkillName::parse(row.get::<_, String>("skill_name")?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
    ))
}

/// Loads the current exact source state only when it is marked available.
pub(super) fn load_active_source(
    connection: &Connection,
    selection_key: &SkillSelectionKey,
) -> Result<Option<DesiredSkillState>, DatabaseError> {
    connection
        .query_row(
            "SELECT sources.source_kind, sources.namespace, sources.identifier AS skill_name,
                    json_extract(revisions.payload_json, '$.display_name') AS display_name,
                    revisions.revision AS source_version, revisions.state_digest AS skill_md_digest
             FROM effect_sources sources
             JOIN effect_source_heads heads ON heads.source_id = sources.id
             JOIN effect_source_revisions revisions ON revisions.id = heads.revision_id
             WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?1
               AND sources.namespace = ?2 AND sources.identifier = ?3
               AND sources.lifecycle = 'active' AND revisions.availability = 'available'",
            params![
                source_kind_value(selection_key.source_kind),
                selection_key.namespace.as_ref(),
                selection_key.name.canonical(),
            ],
            |row| {
                map_desired(row)
                    .map(|(_, desired)| desired)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
            },
        )
        .optional()
        .map_err(Into::into)
}

/// Inserts one already normalized Desired row.
pub(super) fn insert_desired(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    selection_key: &SkillSelectionKey,
    _desired: &DesiredSkillState,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO workspace_effect_desired_items (
             id, workspace_id, source_id, revision_id, created_at, updated_at
         )
         SELECT ?1, ?2, sources.id, heads.revision_id, ?6, ?6
         FROM effect_sources sources
         JOIN effect_source_heads heads ON heads.source_id = sources.id
         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?3
           AND sources.namespace = ?4 AND sources.identifier = ?5",
        params![
            Uuid::new_v4().to_string(),
            workspace_id.as_ref(),
            source_kind_value(selection_key.source_kind),
            selection_key.namespace.as_ref(),
            selection_key.name.canonical(),
            updated_at,
        ],
    )?;
    Ok(())
}

/// Upserts one request per active physical surface at the newest generation.
pub(super) fn enqueue_workspace_surfaces(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    generation: Generation,
    requested_at: i64,
) -> Result<(), DatabaseError> {
    let mut statement = transaction.prepare(
        "SELECT id FROM effect_surfaces
         WHERE workspace_id = ?1 AND lifecycle = 'active'",
    )?;
    let keys = statement
        .query_map(params![workspace_id.as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for key in keys {
        transaction.execute(
            "UPDATE effect_surface_status
             SET desired_generation = MAX(desired_generation, ?2),
                 status_version = status_version + 1, updated_at = ?3
             WHERE surface_id = ?1",
            params![&key, generation_to_sql(generation)?, requested_at],
        )?;
        upsert_reconcile_request(transaction, workspace_id, &key, generation, requested_at)?;
    }
    Ok(())
}

/// Adds a newly published Skill source to every existing Workspace's complete Desired set.
pub(super) fn install_source_in_all_workspaces(
    transaction: &Transaction<'_>,
    source_id: &str,
    revision_id: &str,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    let mut statement = transaction.prepare(
        "SELECT effects.workspace_id, effects.generation
         FROM workspace_effects effects
         WHERE NOT EXISTS (
             SELECT 1 FROM workspace_effect_desired_items desired
             WHERE desired.workspace_id = effects.workspace_id AND desired.source_id = ?1
         )
         ORDER BY effects.workspace_id",
    )?;
    let workspaces = statement
        .query_map(params![source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (workspace_id, current_generation) in workspaces {
        let generation = generation_from_sql(current_generation)?
            .next()
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        transaction.execute(
            "INSERT INTO workspace_effect_desired_items (
                 id, workspace_id, source_id, revision_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                Uuid::new_v4().to_string(),
                &workspace_id,
                source_id,
                revision_id,
                updated_at,
            ],
        )?;
        transaction.execute(
            "UPDATE workspace_effects
             SET generation = ?2, updated_at = ?3 WHERE workspace_id = ?1",
            params![&workspace_id, generation_to_sql(generation)?, updated_at],
        )?;
        enqueue_workspace_surfaces(
            transaction,
            &WorkspaceId::new(workspace_id),
            generation,
            updated_at,
        )?;
    }
    Ok(())
}

/// Removes one retired Skill source from every Workspace while preserving Managed ownership.
pub(super) fn uninstall_source_from_all_workspaces(
    transaction: &Transaction<'_>,
    source_id: &str,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    let mut statement = transaction.prepare(
        "SELECT effects.workspace_id, effects.generation, effects.updated_at
         FROM workspace_effects effects
         JOIN workspace_effect_desired_items desired
           ON desired.workspace_id = effects.workspace_id
         WHERE desired.source_id = ?1
         ORDER BY effects.workspace_id",
    )?;
    let workspaces = statement
        .query_map(params![source_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (workspace_id, current_generation, previous_updated_at) in workspaces {
        let generation = generation_from_sql(current_generation)?
            .next()
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        let effective_updated_at = if updated_at == 0 {
            previous_updated_at
        } else {
            updated_at
        };
        transaction.execute(
            "DELETE FROM workspace_effect_desired_items
             WHERE workspace_id = ?1 AND source_id = ?2",
            params![&workspace_id, source_id],
        )?;
        transaction.execute(
            "UPDATE workspace_effects
             SET generation = ?2, updated_at = ?3 WHERE workspace_id = ?1",
            params![
                &workspace_id,
                generation_to_sql(generation)?,
                effective_updated_at,
            ],
        )?;
        enqueue_workspace_surfaces(
            transaction,
            &WorkspaceId::new(workspace_id),
            generation,
            effective_updated_at,
        )?;
    }
    Ok(())
}

/// Coalesces a surface wakeup without losing the latest requested generation.
pub(super) fn upsert_reconcile_request(
    transaction: &Transaction<'_>,
    _workspace_id: &WorkspaceId,
    surface_key: &str,
    generation: Generation,
    requested_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_reconcile_requests (
             surface_id, requested_generation, request_token, attempt_count,
             requested_at, not_before_at, updated_at
         ) VALUES (?1, ?2, ?3, 0, ?4, ?4, ?4)
         ON CONFLICT(surface_id) DO UPDATE SET
             requested_generation = MAX(requested_generation, excluded.requested_generation),
             request_token = excluded.request_token, attempt_count = 0,
             requested_at = excluded.requested_at, not_before_at = excluded.not_before_at,
             updated_at = excluded.updated_at",
        params![
            surface_key,
            generation_to_sql(generation)?,
            Uuid::new_v4().to_string(),
            requested_at,
        ],
    )?;
    Ok(())
}

/// Coalesces sequential upstream updates by stable selection identity.
pub(super) fn upsert_propagation_request(
    transaction: &Transaction<'_>,
    selection_key: &SkillSelectionKey,
    version: &SourceVersion,
    requested_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_propagation_requests (
             source_id, head_revision_id, request_token, attempt_count,
             requested_at, not_before_at, updated_at
         )
         SELECT sources.id, revisions.id, ?5, 0, ?6, ?6, ?6
         FROM effect_sources sources
         JOIN effect_source_revisions revisions ON revisions.source_id = sources.id
         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?1
           AND sources.namespace = ?2 AND sources.identifier = ?3 AND revisions.revision = ?4
         ON CONFLICT(source_id) DO UPDATE SET
             head_revision_id = excluded.head_revision_id, request_token = excluded.request_token,
             attempt_count = 0, requested_at = excluded.requested_at,
             not_before_at = excluded.not_before_at, updated_at = excluded.updated_at",
        params![
            source_kind_value(selection_key.source_kind),
            selection_key.namespace.as_ref(),
            selection_key.name.canonical(),
            version.as_str(),
            Uuid::new_v4().to_string(),
            requested_at,
        ],
    )?;
    Ok(())
}

/// Lists Workspaces that currently reference a stable source selection.
pub(super) fn referenced_workspaces(
    connection: &Connection,
    selection_key: &SkillSelectionKey,
) -> Result<Vec<WorkspaceId>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT desired.workspace_id
         FROM workspace_effect_desired_items desired
         JOIN effect_sources sources ON sources.id = desired.source_id
         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?1
           AND sources.namespace = ?2 AND sources.identifier = ?3
         ORDER BY desired.workspace_id",
    )?;
    statement
        .query_map(
            params![
                source_kind_value(selection_key.source_kind),
                selection_key.namespace.as_ref(),
                selection_key.name.canonical(),
            ],
            |row| Ok(WorkspaceId::new(row.get::<_, String>(0)?)),
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// Writes a complete ownership ledger and advances only that resource's generation.
pub(super) fn save_managed(
    connection: &Connection,
    managed: &ManagedSkill,
) -> Result<(), DatabaseError> {
    let version = source_version(&managed.state)?;
    let target_json = serde_json::json!({ "target_name": managed.target_name.as_str() });
    connection.execute(
        "INSERT INTO effect_managed_items (
             id, surface_id, source_id, applied_revision_id, target_key, target_json,
             applied_fingerprint, applied_generation, created_at, updated_at
         )
         SELECT ?1, ?2, sources.id, revisions.id, ?6, ?7, ?8, ?9,
                CAST(unixepoch('subsec') * 1000 AS INTEGER),
                CAST(unixepoch('subsec') * 1000 AS INTEGER)
         FROM effect_sources sources
         JOIN effect_source_revisions revisions ON revisions.source_id = sources.id
         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?3
           AND sources.namespace = ?4 AND sources.identifier = ?5 AND revisions.revision = ?10
         ON CONFLICT(id) DO UPDATE SET
             applied_revision_id = excluded.applied_revision_id,
             applied_fingerprint = excluded.applied_fingerprint,
             applied_generation = excluded.applied_generation,
             updated_at = excluded.updated_at",
        params![
            managed.managed_identity.as_str(),
            managed.surface_key.as_str(),
            source_kind_value(managed.selection_key.source_kind),
            managed.selection_key.namespace.as_ref(),
            managed.selection_key.name.canonical(),
            &managed.locator,
            target_json.to_string(),
            managed.applied_fingerprint.as_str(),
            generation_to_sql(managed.applied_generation)?,
            version.as_str(),
        ],
    )?;
    Ok(())
}

/// Reconstructs a managed ledger while validating all strong values.
pub(super) fn map_managed(row: &Row<'_>) -> Result<ManagedSkill, DatabaseError> {
    let (selection_key, state) = map_desired(row)?;
    let target_json: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>("target_json")?).map_err(effect_json_error)?;
    let target_name = target_json
        .get("target_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            DatabaseError::CorruptEffectState("managed target name is missing".to_string())
        })?;
    Ok(ManagedSkill {
        managed_identity: ManagedIdentity::new(row.get::<_, String>("id")?),
        workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        surface_key: ora_effect::SurfaceKey::new(row.get::<_, String>("surface_id")?),
        selection_key,
        locator: row.get("target_key")?,
        target_name: SkillName::parse(target_name)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        state,
        applied_fingerprint: AppliedFingerprint::parse(
            row.get::<_, String>("applied_fingerprint")?,
        )
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        applied_generation: generation_from_sql(row.get("applied_generation")?)?,
    })
}

/// Inserts one operation exactly once before its filesystem side effect.
pub(super) fn insert_operation(
    connection: &Connection,
    operation: &EffectOperation,
) -> Result<(), DatabaseError> {
    let payload = serde_json::to_string(operation).map_err(effect_json_error)?;
    connection.execute(
        "INSERT INTO effect_operations (
             id, surface_id, generation, target_key, operation_kind, phase,
             payload_version, payload_json, prepared_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7,
                   CAST(unixepoch('subsec') * 1000 AS INTEGER),
                   CAST(unixepoch('subsec') * 1000 AS INTEGER))",
        params![
            operation.operation_id.as_str(),
            operation.surface_key.as_str(),
            generation_to_sql(operation.generation)?,
            &operation.locator,
            operation_kind_value(operation.kind),
            operation_phase_value(operation.phase),
            payload,
        ],
    )?;
    if let OperationState::Present(fingerprint) = &operation.planned_state {
        insert_operation_artifact(
            connection,
            operation,
            "staging",
            &operation.staging_path,
            fingerprint,
        )?;
    }
    if let OperationState::Present(fingerprint) = &operation.previous_state {
        insert_operation_artifact(
            connection,
            operation,
            "backup",
            &operation.backup_path,
            fingerprint,
        )?;
    }
    Ok(())
}

/// Reserves one exact recovery resource in the same transaction as Prepared intent.
fn insert_operation_artifact(
    connection: &Connection,
    operation: &EffectOperation,
    artifact_role: &str,
    path: &std::path::Path,
    expected_fingerprint: &AppliedFingerprint,
) -> Result<(), DatabaseError> {
    let locator_key = path.to_string_lossy();
    let locator_json = serde_json::json!({ "path": locator_key }).to_string();
    connection.execute(
        "INSERT INTO effect_operation_artifacts (
             id, operation_id, artifact_role, locator_kind, locator_key, locator_json,
             expected_fingerprint, state, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'filesystem_path', ?4, ?5, ?6, 'reserved',
                   CAST(unixepoch('subsec') * 1000 AS INTEGER),
                   CAST(unixepoch('subsec') * 1000 AS INTEGER))",
        params![
            Uuid::new_v4().to_string(),
            operation.operation_id.as_str(),
            artifact_role,
            locator_key,
            locator_json,
            expected_fingerprint.as_str(),
        ],
    )?;
    Ok(())
}

/// Advances one existing operation phase and keeps its full recovery payload synchronized.
pub(super) fn update_operation(
    connection: &Connection,
    operation: &EffectOperation,
) -> Result<(), DatabaseError> {
    let payload = serde_json::to_string(operation).map_err(effect_json_error)?;
    let updated = connection.execute(
        "UPDATE effect_operations
         SET phase = ?2, payload_json = ?3,
             applied_at = CASE WHEN ?2 IN ('applied', 'finalized')
                               THEN COALESCE(applied_at, CAST(unixepoch('subsec') * 1000 AS INTEGER))
                               ELSE applied_at END,
             finalized_at = CASE WHEN ?2 = 'finalized'
                                 THEN COALESCE(finalized_at, CAST(unixepoch('subsec') * 1000 AS INTEGER))
                                 ELSE finalized_at END,
             updated_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
         WHERE id = ?1",
        params![
            operation.operation_id.as_str(),
            operation_phase_value(operation.phase),
            payload,
        ],
    )?;
    if updated != 1 {
        return Err(DatabaseError::CorruptEffectState(
            "durable operation is missing during phase update".to_string(),
        ));
    }
    match operation.phase {
        EffectOperationPhase::Prepared => {}
        EffectOperationPhase::Applied => {
            connection.execute(
                "UPDATE effect_operation_artifacts
                 SET state = CASE artifact_role
                         WHEN 'staging' THEN 'pending_cleanup'
                         WHEN 'backup' THEN 'retained'
                         ELSE state END,
                     updated_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
                 WHERE operation_id = ?1",
                params![operation.operation_id.as_str()],
            )?;
        }
        EffectOperationPhase::Finalized => {
            connection.execute(
                "UPDATE effect_operation_artifacts
                 SET state = 'pending_cleanup',
                     updated_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
                 WHERE operation_id = ?1",
                params![operation.operation_id.as_str()],
            )?;
        }
    }
    Ok(())
}

/// Maps the status row and its structured current conditions.
pub(super) fn map_surface_status(row: &Row<'_>) -> Result<SurfaceStatus, rusqlite::Error> {
    let conditions_json: String = row.get("conditions_json")?;
    let conditions: Vec<Condition> = serde_json::from_str(&conditions_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(SurfaceStatus {
        workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        surface_key: ora_effect::SurfaceKey::new(row.get::<_, String>("surface_key")?),
        desired_generation: generation_from_row(row, "desired_generation")?,
        observed_generation: generation_from_row(row, "observed_generation")?,
        applied_generation: generation_from_row(row, "applied_generation")?,
        phase: parse_surface_phase(&row.get::<_, String>("phase")?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        revision: u64::try_from(row.get::<_, i64>("revision")?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        updated_at: row.get("updated_at")?,
        conditions,
    })
}

/// Loads normalized current conditions for one Surface or one of its Consumers.
pub(super) fn load_conditions(
    connection: &Connection,
    surface_id: &str,
    consumer_id: Option<&str>,
) -> Result<Vec<Condition>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT subject_id, reason, message, first_observed_at, last_observed_at,
                failed_generation
         FROM effect_conditions
         WHERE surface_id = ?1 AND consumer_id IS ?2
         ORDER BY subject_kind, subject_id, reason",
    )?;
    let rows = statement.query_map(params![surface_id, consumer_id], |row| {
        let subject_json: String = row.get(0)?;
        let reason: String = row.get(1)?;
        let subject = serde_json::from_str::<ConditionSubject>(&subject_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let reason =
            serde_json::from_str::<ConditionReason>(&format!("\"{reason}\"")).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        Ok(Condition {
            subject,
            reason,
            message: row.get(2)?,
            first_occurred_at: row.get(3)?,
            last_occurred_at: row.get(4)?,
            failed_generation: generation_from_sql(row.get(5)?).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })?,
            retry_policy: reason.retry_policy(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Replaces one status scope's normalized conditions after its version is persisted.
pub(super) fn replace_conditions(
    transaction: &Transaction<'_>,
    surface_id: &str,
    consumer_id: Option<&str>,
    conditions: &[Condition],
) -> Result<(), DatabaseError> {
    transaction.execute(
        "DELETE FROM effect_conditions WHERE surface_id = ?1 AND consumer_id IS ?2",
        params![surface_id, consumer_id],
    )?;
    for condition in conditions {
        let subject_kind = match &condition.subject {
            ConditionSubject::DesiredSkill { .. } => "desired_item",
            ConditionSubject::ManagedSkill { .. } => "managed_item",
            ConditionSubject::Surface { .. } => "surface",
            ConditionSubject::Consumer { .. } => "consumer",
        };
        let subject_id = serde_json::to_string(&condition.subject).map_err(effect_json_error)?;
        let reason_json = serde_json::to_string(&condition.reason).map_err(effect_json_error)?;
        let reason = reason_json.trim_matches('"');
        transaction.execute(
            "INSERT INTO effect_conditions (
                 id, surface_id, consumer_id, subject_kind, subject_id, reason,
                 failed_generation, message, first_observed_at, last_observed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                Uuid::new_v4().to_string(),
                surface_id,
                consumer_id,
                subject_kind,
                subject_id,
                reason,
                generation_to_sql(condition.failed_generation)?,
                &condition.message,
                condition.first_occurred_at,
                condition.last_occurred_at,
            ],
        )?;
    }
    Ok(())
}

/// Converts a checked generation column inside a rusqlite row mapper.
pub(super) fn generation_from_row(
    row: &Row<'_>,
    column: &str,
) -> Result<Generation, rusqlite::Error> {
    let value: i64 = row.get(column)?;
    generation_from_sql(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

/// Extracts the exact source revision from catalog-backed desired state.
pub(super) fn source_version(desired: &DesiredSkillState) -> Result<&SourceVersion, DatabaseError> {
    desired.state().source.version().ok_or_else(|| {
        DatabaseError::CorruptEffectState("preserved state entered persistence".to_string())
    })
}

/// Reconstructs the stable selection identity embedded in a desired state.
pub(super) fn source_selection(
    desired: &DesiredSkillState,
) -> Result<SkillSelectionKey, DatabaseError> {
    desired
        .state()
        .source
        .selection_key(desired.state().name.clone())
        .ok_or_else(|| {
            DatabaseError::CorruptEffectState("preserved state entered persistence".to_string())
        })
}

pub(super) fn source_kind_value(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Local => "local",
        SourceKind::Plugin => "plugin",
    }
}

pub(super) fn parse_source_kind(value: &str) -> Result<SourceKind, DatabaseError> {
    match value {
        "local" => Ok(SourceKind::Local),
        "plugin" => Ok(SourceKind::Plugin),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown source kind".to_string(),
        )),
    }
}

pub(super) fn operation_kind_value(kind: ora_effect::EffectOperationKind) -> &'static str {
    match kind {
        ora_effect::EffectOperationKind::Create => "create",
        ora_effect::EffectOperationKind::Update => "update",
        ora_effect::EffectOperationKind::Replace => "replace",
        ora_effect::EffectOperationKind::Delete => "delete",
    }
}

pub(super) fn operation_phase_value(phase: EffectOperationPhase) -> &'static str {
    match phase {
        EffectOperationPhase::Prepared => "prepared",
        EffectOperationPhase::Applied => "applied",
        EffectOperationPhase::Finalized => "finalized",
    }
}

pub(super) fn surface_phase_value(phase: SurfacePhase) -> &'static str {
    match phase {
        SurfacePhase::Pending => "pending",
        SurfacePhase::WaitingForIdle => "waiting_for_idle",
        SurfacePhase::Quiescing => "quiescing",
        SurfacePhase::Applying => "applying",
        SurfacePhase::Resuming => "resuming",
        SurfacePhase::Current => "current",
        SurfacePhase::Degraded => "degraded",
        SurfacePhase::Retiring => "retiring",
        SurfacePhase::RecoveryRequired => "recovery_required",
    }
}

pub(super) fn parse_surface_phase(value: &str) -> Result<SurfacePhase, DatabaseError> {
    match value {
        "pending" => Ok(SurfacePhase::Pending),
        "waiting_for_idle" => Ok(SurfacePhase::WaitingForIdle),
        "quiescing" => Ok(SurfacePhase::Quiescing),
        "applying" => Ok(SurfacePhase::Applying),
        "resuming" => Ok(SurfacePhase::Resuming),
        "current" => Ok(SurfacePhase::Current),
        "degraded" => Ok(SurfacePhase::Degraded),
        "retiring" => Ok(SurfacePhase::Retiring),
        "recovery_required" => Ok(SurfacePhase::RecoveryRequired),
        _ => Err(DatabaseError::CorruptEffectState(
            "unknown surface phase".to_string(),
        )),
    }
}

/// Converts the unsigned domain value into SQLite's signed integer without truncation.
pub(super) fn generation_to_sql(generation: Generation) -> Result<i64, DatabaseError> {
    u64_to_sql(generation.value(), "generation")
}

pub(super) fn u64_to_sql(value: u64, field: &str) -> Result<i64, DatabaseError> {
    i64::try_from(value).map_err(|_| {
        DatabaseError::CorruptEffectState(format!("{field} exceeds SQLite integer range"))
    })
}

pub(super) fn generation_from_sql(value: i64) -> Result<Generation, DatabaseError> {
    u64::try_from(value)
        .map(Generation::new)
        .map_err(|_| DatabaseError::CorruptEffectState("negative generation".to_string()))
}

pub(super) fn effect_json_error(error: serde_json::Error) -> DatabaseError {
    DatabaseError::CorruptEffectState(error.to_string())
}

pub(super) fn effect_repository_error(error: DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

pub(super) fn source_provider_error(error: DatabaseError) -> SourceError {
    SourceError::Provider {
        source: Box::new(error),
    }
}
