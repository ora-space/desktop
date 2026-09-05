use crate::DatabaseError;
use ora_domain::WorkspaceId;
use ora_effect::{
    ConsumerIdentity, ConsumerKind, ConsumerRevision, ConsumerRevisionId, DesiredEffect,
    DesiredEffectIdentity, DesiredState, Digest, EffectMutation, EffectResource, EffectResourceId,
    EffectRevision, EffectRevisionId, EffectScopeId, EffectTarget, EffectTargetId, Generation,
    ResourceAdapterIdentity, ResourceKey, ResourceLifecycle, ResourcePhase, ResourceStatus,
    RevisionAvailability, StableReason, StatusVersion, TargetDeclaration, TargetLifecycle,
    TargetPhase, TargetProgress, TargetStatus, ValidatedEffectDefinition,
    ValidatedEffectParameters, VersionedResourceDescriptor,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;

/// Serializes versioned Effect values for columns that Core never interprets as arbitrary JSON.
pub(super) fn effect_json(value: &impl Serialize) -> Result<String, DatabaseError> {
    serde_json::to_string(value)
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))
}

/// Deserializes a column into the exact typed contract selected by its version columns.
pub(super) fn parse_effect_json<T: DeserializeOwned>(value: String) -> Result<T, DatabaseError> {
    serde_json::from_str(&value)
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))
}

/// Converts a Generation into SQLite's signed integer without truncation.
pub(super) fn generation_to_sql(generation: Generation) -> Result<i64, DatabaseError> {
    i64::try_from(generation.value()).map_err(|_| {
        DatabaseError::CorruptEffectState("Effect generation exceeds SQLite INTEGER".to_string())
    })
}

/// Restores a non-negative SQLite generation.
pub(super) fn generation_from_sql(value: i64) -> Result<Generation, DatabaseError> {
    u64::try_from(value)
        .map(Generation::new)
        .map_err(|_| DatabaseError::CorruptEffectState("Effect generation is negative".to_string()))
}

/// Converts a positive status version into SQLite's signed integer.
pub(super) fn status_version_to_sql(value: StatusVersion) -> Result<i64, DatabaseError> {
    i64::try_from(value.value()).map_err(|_| {
        DatabaseError::CorruptEffectState(
            "Effect status version exceeds SQLite INTEGER".to_string(),
        )
    })
}

/// Loads one complete Desired State and its current Scope generation.
pub(super) fn load_desired_state(
    connection: &Connection,
    scope: &EffectScopeId,
) -> Result<DesiredState, DatabaseError> {
    let scope_id = scope.storage_key();
    let generation = connection
        .query_row(
            "SELECT generation FROM effect_scopes WHERE id = ?1",
            params![&scope_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| {
            DatabaseError::CorruptEffectState(format!("missing Effect Scope {scope}"))
        })?;
    let mut statement = connection.prepare(
        "SELECT id, revision_id, parameters_json, selector_json
         FROM effect_desired_effects WHERE scope_id = ?1 ORDER BY id",
    )?;
    let rows = statement.query_map(params![&scope_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut effects = BTreeMap::new();
    for row in rows {
        let (id, revision_id, parameters, selector) = row?;
        let identity = DesiredEffectIdentity::new(id);
        effects.insert(
            identity.clone(),
            DesiredEffect {
                identity,
                revision: EffectRevisionId::new(revision_id),
                parameters: parse_effect_json::<ValidatedEffectParameters>(parameters)?,
                audience: parse_effect_json(selector)?,
            },
        );
    }
    Ok(DesiredState {
        scope: scope.clone(),
        generation: generation_from_sql(generation)?,
        effects,
    })
}

/// Loads every immutable revision referenced by the current Desired State.
pub(super) fn load_revisions_for_scope(
    connection: &Connection,
    scope: &EffectScopeId,
) -> Result<BTreeMap<EffectRevisionId, EffectRevision>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT revision.id, revision.source_id, revision.revision_key,
                revision.definition_json, revision.digest, revision.availability,
                revision.unavailable_reason
         FROM effect_revisions revision
         WHERE revision.id IN (
             SELECT desired.revision_id FROM effect_desired_effects desired
             WHERE desired.scope_id = ?1
         ) ORDER BY revision.id",
    )?;
    let mut rows = statement.query(params![scope.storage_key()])?;
    let mut revisions = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let revision = map_revision(row)?;
        revisions.insert(revision.identity.clone(), revision);
    }
    Ok(revisions)
}

/// Reconstructs one immutable Effect Revision from normalized columns.
fn map_revision(row: &Row<'_>) -> Result<EffectRevision, DatabaseError> {
    let availability = match row.get::<_, String>("availability")?.as_str() {
        "available" => RevisionAvailability::Available,
        "unavailable" => RevisionAvailability::Unavailable(
            StableReason::parse(
                row.get::<_, Option<String>>("unavailable_reason")?
                    .ok_or_else(|| {
                        DatabaseError::CorruptEffectState(
                            "unavailable Effect revision lacks a stable reason".to_string(),
                        )
                    })?,
            )
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        ),
        other => {
            return Err(DatabaseError::CorruptEffectState(format!(
                "unknown Effect revision availability {other}"
            )));
        }
    };
    Ok(EffectRevision {
        identity: EffectRevisionId::new(row.get::<_, String>("id")?),
        source: ora_effect::EffectSourceIdentity::new(row.get::<_, String>("source_id")?),
        revision_key: ora_effect::SourceRevisionKey::parse(row.get::<_, String>("revision_key")?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        definition: parse_effect_json::<ValidatedEffectDefinition>(
            row.get::<_, String>("definition_json")?,
        )?,
        digest: Digest::parse(row.get::<_, String>("digest")?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        availability,
    })
}

/// Loads the stable Target and its exact current Consumer Revision.
pub(super) fn load_target(
    connection: &Connection,
    target_id: &EffectTargetId,
) -> Result<EffectTarget, DatabaseError> {
    connection
        .query_row(
            "SELECT target.id, target.scope_id, target.consumer_revision_id, target.lifecycle,
                consumer.consumer_kind, consumer.identity_key
         FROM effect_targets target
         JOIN effect_consumers consumer ON consumer.id = target.consumer_id
         WHERE target.id = ?1",
            params![target_id.as_str()],
            |row| {
                let scope_id = row.get::<_, String>("scope_id")?;
                let workspace_id = scope_id.strip_prefix("workspace:").ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "scope_id".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;
                let consumer_kind = ConsumerKind::parse(row.get::<_, String>("consumer_kind")?)
                    .map_err(to_sql_conversion_error)?;
                let consumer =
                    ConsumerIdentity::new(consumer_kind, row.get::<_, String>("identity_key")?)
                        .map_err(to_sql_conversion_error)?;
                let lifecycle = parse_target_lifecycle(&row.get::<_, String>("lifecycle")?)
                    .map_err(to_sql_conversion_error)?;
                Ok(EffectTarget {
                    identity: EffectTargetId::new(row.get::<_, String>("id")?),
                    scope: EffectScopeId::Workspace(WorkspaceId::new(workspace_id)),
                    consumer,
                    consumer_revision: ConsumerRevisionId::new(
                        row.get::<_, String>("consumer_revision_id")?,
                    ),
                    lifecycle,
                })
            },
        )
        .map_err(Into::into)
}

/// Loads one immutable Consumer capability snapshot.
pub(super) fn load_consumer_revision(
    connection: &Connection,
    revision_id: &ConsumerRevisionId,
) -> Result<ConsumerRevision, DatabaseError> {
    connection
        .query_row(
            "SELECT revision.id, revision.capabilities_json, revision.declaration_digest,
                consumer.consumer_kind, consumer.identity_key
         FROM effect_consumer_revisions revision
         JOIN effect_consumers consumer ON consumer.id = revision.consumer_id
         WHERE revision.id = ?1",
            params![revision_id.as_str()],
            |row| {
                let kind = ConsumerKind::parse(row.get::<_, String>("consumer_kind")?)
                    .map_err(to_sql_conversion_error)?;
                let consumer = ConsumerIdentity::new(kind, row.get::<_, String>("identity_key")?)
                    .map_err(to_sql_conversion_error)?;
                let capabilities =
                    serde_json::from_str(&row.get::<_, String>("capabilities_json")?)
                        .map_err(to_sql_conversion_error)?;
                let declaration_digest = Digest::parse(row.get::<_, String>("declaration_digest")?)
                    .map_err(to_sql_conversion_error)?;
                Ok(ConsumerRevision {
                    identity: ConsumerRevisionId::new(row.get::<_, String>("id")?),
                    consumer,
                    capabilities,
                    declaration_digest,
                })
            },
        )
        .map_err(Into::into)
}

/// Loads the complete replaceable declaration snapshot for one Target.
pub(super) fn load_target_declaration(
    connection: &Connection,
    target_id: &EffectTargetId,
) -> Result<TargetDeclaration, DatabaseError> {
    let json = connection.query_row(
        "SELECT declaration_json FROM effect_target_declarations WHERE target_id = ?1",
        params![target_id.as_str()],
        |row| row.get::<_, String>(0),
    )?;
    parse_effect_json(json)
}

/// Loads one Resource and its versioned adapter descriptor.
pub(super) fn load_resource(
    connection: &Connection,
    resource_id: &EffectResourceId,
) -> Result<EffectResource, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT resource.id, resource.scope_id, resource.resource_key, resource.adapter_id,
                resource.descriptor_json, resource.materialization_format, resource.lifecycle
         FROM effect_resources resource WHERE resource.id = ?1",
    )?;
    let mut rows = statement.query(params![resource_id.as_str()])?;
    let row = rows.next()?.ok_or_else(|| {
        DatabaseError::CorruptEffectState(format!("missing Effect Resource {resource_id}"))
    })?;
    map_resource(row)
}

/// Reconstructs one Resource row and validates its first-version Workspace Scope.
fn map_resource(row: &Row<'_>) -> Result<EffectResource, DatabaseError> {
    let scope_id = row.get::<_, String>("scope_id")?;
    let workspace_id = scope_id.strip_prefix("workspace:").ok_or_else(|| {
        DatabaseError::CorruptEffectState(format!("unknown Effect Scope identity {scope_id}"))
    })?;
    let lifecycle = match row.get::<_, String>("lifecycle")?.as_str() {
        "active" => ResourceLifecycle::Active,
        "retiring" => ResourceLifecycle::Retiring,
        other => {
            return Err(DatabaseError::CorruptEffectState(format!(
                "unknown Effect Resource lifecycle {other}"
            )));
        }
    };
    Ok(EffectResource {
        identity: EffectResourceId::new(row.get::<_, String>("id")?),
        scope: EffectScopeId::Workspace(WorkspaceId::new(workspace_id)),
        resource_key: ResourceKey::parse(row.get::<_, String>("resource_key")?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        adapter: ResourceAdapterIdentity::parse(row.get::<_, String>("adapter_id")?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        descriptor: parse_effect_json::<VersionedResourceDescriptor>(
            row.get::<_, String>("descriptor_json")?,
        )?,
        format: ora_effect::MaterializationFormat::parse(
            row.get::<_, String>("materialization_format")?,
        )
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        lifecycle,
    })
}

/// Loads and validates one Target status snapshot.
pub(super) fn load_target_status(
    connection: &Connection,
    target_id: &EffectTargetId,
) -> Result<Option<TargetStatus>, DatabaseError> {
    let row = connection
        .query_row(
            "SELECT desired_generation, observed_generation, applied_generation,
                    ready_generation, phase, recovery_operation_id, status_version
             FROM effect_target_status WHERE target_id = ?1",
            params![target_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(desired, observed, applied, ready, phase, recovery, version)| {
            Ok(TargetStatus::restore(
                target_id.clone(),
                TargetProgress::restore(
                    generation_from_sql(desired)?,
                    generation_from_sql(observed)?,
                    generation_from_sql(applied)?,
                    generation_from_sql(ready)?,
                )
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
                parse_target_phase(&phase, recovery)?,
                StatusVersion::new(u64::try_from(version).map_err(|_| {
                    DatabaseError::CorruptEffectState("negative Target status version".to_string())
                })?)
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
            ))
        },
    )
    .transpose()
}

/// Loads and validates one Resource status snapshot.
pub(super) fn load_resource_status(
    connection: &Connection,
    resource_id: &EffectResourceId,
) -> Result<Option<ResourceStatus>, DatabaseError> {
    let row = connection
        .query_row(
            "SELECT desired_generation, observed_generation, applied_generation, phase,
                    recovery_operation_id, status_version
             FROM effect_resource_status WHERE resource_id = ?1",
            params![resource_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    row.map(|(desired, observed, applied, phase, recovery, version)| {
        ResourceStatus::restore(
            resource_id.clone(),
            generation_from_sql(desired)?,
            generation_from_sql(observed)?,
            generation_from_sql(applied)?,
            parse_resource_phase(&phase, recovery)?,
            StatusVersion::new(u64::try_from(version).map_err(|_| {
                DatabaseError::CorruptEffectState("negative Resource status version".to_string())
            })?)
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
        )
        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))
    })
    .transpose()
}

/// Encodes one Target phase while keeping recovery identity in its dedicated column.
pub(super) fn target_phase_value(phase: &TargetPhase) -> (&'static str, Option<&str>) {
    match phase {
        TargetPhase::Pending => ("pending", None),
        TargetPhase::Reconciling(stage) => match stage {
            ora_effect::ReconcileStage::Planning => ("planning", None),
            ora_effect::ReconcileStage::Coordinating => ("coordinating", None),
            ora_effect::ReconcileStage::Applying => ("applying", None),
            ora_effect::ReconcileStage::Verifying => ("verifying", None),
            ora_effect::ReconcileStage::Activating => ("activating", None),
        },
        TargetPhase::Current => ("current", None),
        TargetPhase::CurrentWithIssues => ("current_with_issues", None),
        TargetPhase::Retiring => ("retiring", None),
        TargetPhase::RecoveryRequired(operation) => ("recovery_required", Some(operation.as_str())),
    }
}

/// Encodes one Resource phase while keeping recovery identity in its dedicated column.
pub(super) fn resource_phase_value(phase: &ResourcePhase) -> (&'static str, Option<&str>) {
    match phase {
        ResourcePhase::Pending => ("pending", None),
        ResourcePhase::Reconciling => ("reconciling", None),
        ResourcePhase::Current => ("current", None),
        ResourcePhase::Retiring => ("retiring", None),
        ResourcePhase::RecoveryRequired(operation) => {
            ("recovery_required", Some(operation.as_str()))
        }
    }
}

/// Decodes the Target lifecycle without accepting future values accidentally.
fn parse_target_lifecycle(value: &str) -> Result<TargetLifecycle, DatabaseError> {
    match value {
        "active" => Ok(TargetLifecycle::Active),
        "retiring" => Ok(TargetLifecycle::Retiring),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect Target lifecycle {other}"
        ))),
    }
}

/// Decodes one generic Target phase and its required recovery identity.
fn parse_target_phase(value: &str, recovery: Option<String>) -> Result<TargetPhase, DatabaseError> {
    match value {
        "pending" => Ok(TargetPhase::Pending),
        "planning" => Ok(TargetPhase::Reconciling(
            ora_effect::ReconcileStage::Planning,
        )),
        "coordinating" => Ok(TargetPhase::Reconciling(
            ora_effect::ReconcileStage::Coordinating,
        )),
        "applying" => Ok(TargetPhase::Reconciling(
            ora_effect::ReconcileStage::Applying,
        )),
        "verifying" => Ok(TargetPhase::Reconciling(
            ora_effect::ReconcileStage::Verifying,
        )),
        "activating" => Ok(TargetPhase::Reconciling(
            ora_effect::ReconcileStage::Activating,
        )),
        "current" => Ok(TargetPhase::Current),
        "current_with_issues" => Ok(TargetPhase::CurrentWithIssues),
        "retiring" => Ok(TargetPhase::Retiring),
        "recovery_required" => recovery
            .map(ora_effect::EffectOperationId::new)
            .map(TargetPhase::RecoveryRequired)
            .ok_or_else(|| {
                DatabaseError::CorruptEffectState(
                    "RecoveryRequired Target lacks an operation".to_string(),
                )
            }),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect Target phase {other}"
        ))),
    }
}

/// Decodes one Resource phase and its required recovery identity.
fn parse_resource_phase(
    value: &str,
    recovery: Option<String>,
) -> Result<ResourcePhase, DatabaseError> {
    match value {
        "pending" => Ok(ResourcePhase::Pending),
        "reconciling" => Ok(ResourcePhase::Reconciling),
        "current" => Ok(ResourcePhase::Current),
        "retiring" => Ok(ResourcePhase::Retiring),
        "recovery_required" => recovery
            .map(ora_effect::EffectOperationId::new)
            .map(ResourcePhase::RecoveryRequired)
            .ok_or_else(|| {
                DatabaseError::CorruptEffectState(
                    "RecoveryRequired Resource lacks an operation".to_string(),
                )
            }),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect Resource phase {other}"
        ))),
    }
}

/// Converts domain/JSON construction failures inside a rusqlite row mapper.
fn to_sql_conversion_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

/// Encodes a mutation kind for the immutable operation journal.
pub(super) fn mutation_value(mutation: EffectMutation) -> &'static str {
    match mutation {
        EffectMutation::Create => "create",
        EffectMutation::Update => "update",
        EffectMutation::Replace => "replace",
        EffectMutation::Delete => "delete",
    }
}

/// Decodes a persisted operation mutation kind exhaustively.
pub(super) fn parse_mutation(value: &str) -> Result<EffectMutation, DatabaseError> {
    match value {
        "create" => Ok(EffectMutation::Create),
        "update" => Ok(EffectMutation::Update),
        "replace" => Ok(EffectMutation::Replace),
        "delete" => Ok(EffectMutation::Delete),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect mutation {other}"
        ))),
    }
}
