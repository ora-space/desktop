use crate::DatabaseError;
use ora_domain::{Namespace, WorkspaceId};
use ora_effect::{
    AppliedFingerprint, Condition, DesiredSkillState, Digest, EffectOperation,
    EffectOperationPhase, Generation, ManagedIdentity, ManagedSkill, RepositoryError, SkillName,
    SkillSelectionKey, SkillSource, SkillState, SourceError, SourceKind, SourceVersion,
    SurfacePhase, SurfaceStatus, WorkspaceEffect, WorkspaceEffectSpec,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use std::collections::BTreeMap;

/// Loads the complete normalized desired set while treating a missing row as generation zero.
pub(super) fn load_workspace_effect(
    connection: &Connection,
    workspace_id: &WorkspaceId,
) -> Result<WorkspaceEffect, DatabaseError> {
    let generation = current_generation(connection, workspace_id)?;
    let mut statement = connection.prepare(
        "SELECT source_kind, namespace, skill_name, display_name, source_version, skill_md_digest
         FROM workspace_effect_desired_skills
         WHERE workspace_id = ?1
         ORDER BY source_kind, namespace, skill_name",
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
            "SELECT source_kind, namespace, skill_name, display_name, source_version,
                    skill_md_digest
             FROM effect_source_states
             WHERE source_kind = ?1 AND namespace = ?2 AND skill_name = ?3
               AND availability = 'available'",
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
    desired: &DesiredSkillState,
) -> Result<(), DatabaseError> {
    let version = source_version(desired)?;
    transaction.execute(
        "INSERT INTO workspace_effect_desired_skills (
             workspace_id, source_kind, namespace, skill_name, display_name,
             source_version, skill_md_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            workspace_id.as_ref(),
            source_kind_value(selection_key.source_kind),
            selection_key.namespace.as_ref(),
            selection_key.name.canonical(),
            desired.state().name.as_str(),
            version.as_str(),
            desired.state().skill_md_digest.as_str(),
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
        "SELECT surface_key FROM effect_surfaces
         WHERE workspace_id = ?1 AND lifecycle IN ('active', 'retiring')",
    )?;
    let keys = statement
        .query_map(params![workspace_id.as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for key in keys {
        upsert_reconcile_request(transaction, workspace_id, &key, generation, requested_at)?;
    }
    Ok(())
}

/// Coalesces a surface wakeup without losing the latest requested generation.
pub(super) fn upsert_reconcile_request(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    surface_key: &str,
    generation: Generation,
    requested_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_reconcile_requests (
             workspace_id, surface_key, requested_generation, requested_at
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(workspace_id, surface_key) DO UPDATE SET
             requested_generation = MAX(requested_generation, excluded.requested_generation),
             requested_at = excluded.requested_at",
        params![
            workspace_id.as_ref(),
            surface_key,
            generation_to_sql(generation)?,
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
        "INSERT INTO effect_source_propagation_requests (
             source_kind, namespace, skill_name, requested_version, requested_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source_kind, namespace, skill_name) DO UPDATE SET
             requested_version = excluded.requested_version,
             requested_at = excluded.requested_at",
        params![
            source_kind_value(selection_key.source_kind),
            selection_key.namespace.as_ref(),
            selection_key.name.canonical(),
            version.as_str(),
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
        "SELECT workspace_id FROM workspace_effect_desired_skills
         WHERE source_kind = ?1 AND namespace = ?2 AND skill_name = ?3
         ORDER BY workspace_id",
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
    connection.execute(
        "INSERT INTO effect_managed_skills (
             managed_identity, workspace_id, surface_key, source_kind, namespace, skill_name,
             display_name, source_version, skill_md_digest, locator, target_name,
             applied_fingerprint, applied_generation
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(managed_identity) DO UPDATE SET
             display_name = excluded.display_name,
             source_version = excluded.source_version,
             skill_md_digest = excluded.skill_md_digest,
             locator = excluded.locator,
             target_name = excluded.target_name,
             applied_fingerprint = excluded.applied_fingerprint,
             applied_generation = excluded.applied_generation",
        params![
            managed.managed_identity.as_str(),
            managed.workspace_id.as_ref(),
            managed.surface_key.as_str(),
            source_kind_value(managed.selection_key.source_kind),
            managed.selection_key.namespace.as_ref(),
            managed.selection_key.name.canonical(),
            managed.state.state().name.as_str(),
            version.as_str(),
            managed.state.state().skill_md_digest.as_str(),
            &managed.locator,
            managed.target_name.as_str(),
            managed.applied_fingerprint.as_str(),
            generation_to_sql(managed.applied_generation)?,
        ],
    )?;
    Ok(())
}

/// Reconstructs a managed ledger while validating all strong values.
pub(super) fn map_managed(row: &Row<'_>) -> Result<ManagedSkill, DatabaseError> {
    let (selection_key, state) = map_desired(row)?;
    Ok(ManagedSkill {
        managed_identity: ManagedIdentity::new(row.get::<_, String>("managed_identity")?),
        workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        surface_key: ora_effect::SurfaceKey::new(row.get::<_, String>("surface_key")?),
        selection_key,
        locator: row.get("locator")?,
        target_name: SkillName::parse(row.get::<_, String>("target_name")?)
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
             operation_id, workspace_id, surface_key, generation, locator,
             operation_kind, phase, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            operation.operation_id.as_str(),
            operation.workspace_id.as_ref(),
            operation.surface_key.as_str(),
            generation_to_sql(operation.generation)?,
            &operation.locator,
            operation_kind_value(operation.kind),
            operation_phase_value(operation.phase),
            payload,
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
        "UPDATE effect_operations SET phase = ?2, payload_json = ?3 WHERE operation_id = ?1",
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
