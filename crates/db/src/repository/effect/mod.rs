mod mapping;

use crate::{DatabaseError, RepositoryPool};
use mapping::*;
use ora_domain::WorkspaceId;
use ora_effect::{
    ConsumerStatus, DesiredSkillState, Digest, EffectOperation, EffectRepository, Generation,
    LedgerTransition, ManagedIdentity, ManagedSkill, ReplaceEffectOutcome, RepositoryError,
    SkillSelectionKey, SourceError, SourceProvider, SourceSnapshot, SourceVersion,
    SurfaceDescriptorSet, SurfaceStatus, WorkspaceEffect, WorkspaceEffectSpec,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::fs;
use std::path::Path;

/// Selects whether publishing a source revision should wake existing Desired references.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcePublication {
    Create,
    Update,
}

/// Result of attempting to remove an active source under the same write lock as Desired writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceMutationOutcome {
    Deleted,
    Missing,
    InUse { workspace_ids: Vec<WorkspaceId> },
}

/// SQLite implementation of Effect's normalized durable state boundary.
#[derive(Clone, Debug)]
pub struct SqliteEffectRepository {
    pool: RepositoryPool,
}

impl SqliteEffectRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Publishes a validated source revision and atomically coalesces its propagation wakeup.
    pub fn publish_source(
        &self,
        source: &DesiredSkillState,
        package_root: &Path,
        publication: SourcePublication,
        updated_at: i64,
    ) -> Result<(), DatabaseError> {
        let key = source_selection(source)?;
        let version = source_version(source)?;
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "INSERT INTO effect_source_states (
                     source_kind, namespace, skill_name, display_name, source_version,
                     skill_md_digest, package_root, availability, unavailable_reason, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'available', NULL, ?8)
                 ON CONFLICT(source_kind, namespace, skill_name) DO UPDATE SET
                     display_name = excluded.display_name,
                     source_version = excluded.source_version,
                     skill_md_digest = excluded.skill_md_digest,
                     package_root = excluded.package_root,
                     availability = 'available',
                     unavailable_reason = NULL,
                     updated_at = excluded.updated_at",
                params![
                    source_kind_value(key.source_kind),
                    key.namespace.as_ref(),
                    key.name.canonical(),
                    key.name.as_str(),
                    version.as_str(),
                    source.state().skill_md_digest.as_str(),
                    package_root.to_string_lossy(),
                    updated_at,
                ],
            )?;
            if publication == SourcePublication::Update {
                upsert_propagation_request(&transaction, &key, version, updated_at)?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    /// Keeps a catalog source visible while preventing new Desired selections after drift.
    pub fn mark_source_unavailable(
        &self,
        selection_key: &SkillSelectionKey,
        reason: &str,
        updated_at: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE effect_source_states
                 SET availability = 'unavailable', unavailable_reason = ?4, updated_at = ?5
                 WHERE source_kind = ?1 AND namespace = ?2 AND skill_name = ?3",
                params![
                    source_kind_value(selection_key.source_kind),
                    selection_key.namespace.as_ref(),
                    selection_key.name.canonical(),
                    reason,
                    updated_at,
                ],
            )?;
            Ok(changed > 0)
        })
    }

    /// Removes a source only when an immediate transaction proves no Desired row references it.
    pub fn delete_source(
        &self,
        selection_key: &SkillSelectionKey,
    ) -> Result<SourceMutationOutcome, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let workspace_ids = referenced_workspaces(&transaction, selection_key)?;
            if !workspace_ids.is_empty() {
                transaction.commit()?;
                return Ok(SourceMutationOutcome::InUse { workspace_ids });
            }
            let deleted = transaction.execute(
                "DELETE FROM effect_source_states
                 WHERE source_kind = ?1 AND namespace = ?2 AND skill_name = ?3",
                params![
                    source_kind_value(selection_key.source_kind),
                    selection_key.namespace.as_ref(),
                    selection_key.name.canonical(),
                ],
            )?;
            transaction.commit()?;
            Ok(if deleted == 0 {
                SourceMutationOutcome::Missing
            } else {
                SourceMutationOutcome::Deleted
            })
        })
    }

    /// Replaces the persisted consumer snapshot and retires physical surfaces no longer declared.
    pub fn replace_surfaces(
        &self,
        workspace_id: &WorkspaceId,
        workspace_path: &Path,
        surfaces: &[SurfaceDescriptorSet],
        updated_at: i64,
    ) -> Result<(), DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let generation = current_generation(&transaction, workspace_id)?;
            let active_keys = surfaces
                .iter()
                .map(|surface| surface.surface_key.as_str().to_string())
                .collect::<Vec<_>>();
            {
                let mut statement = transaction.prepare(
                    "SELECT surface_key FROM effect_surfaces
                     WHERE workspace_id = ?1 AND lifecycle = 'active'",
                )?;
                let existing = statement
                    .query_map(params![workspace_id.as_ref()], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for surface_key in existing {
                    if active_keys.contains(&surface_key) {
                        continue;
                    }
                    transaction.execute(
                        "UPDATE effect_surfaces SET lifecycle = 'retiring', updated_at = ?3
                         WHERE workspace_id = ?1 AND surface_key = ?2",
                        params![workspace_id.as_ref(), surface_key, updated_at],
                    )?;
                    upsert_reconcile_request(
                        &transaction,
                        workspace_id,
                        &surface_key,
                        generation,
                        updated_at,
                    )?;
                }
            }
            for surface in surfaces {
                let consumers_json =
                    serde_json::to_string(&surface.consumers).map_err(effect_json_error)?;
                transaction.execute(
                    "INSERT INTO effect_surfaces (
                         workspace_id, surface_key, workspace_path, relative_path,
                         materialization_format, lifecycle, consumers_json, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?7)
                     ON CONFLICT(workspace_id, surface_key) DO UPDATE SET
                         workspace_path = excluded.workspace_path,
                         relative_path = excluded.relative_path,
                         materialization_format = excluded.materialization_format,
                         lifecycle = 'active',
                         consumers_json = excluded.consumers_json,
                         updated_at = excluded.updated_at",
                    params![
                        workspace_id.as_ref(),
                        surface.surface_key.as_str(),
                        workspace_path.to_string_lossy(),
                        surface.path.as_str(),
                        surface.format.as_str(),
                        consumers_json,
                        updated_at,
                    ],
                )?;
                upsert_reconcile_request(
                    &transaction,
                    workspace_id,
                    surface.surface_key.as_str(),
                    generation,
                    updated_at,
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    /// Advances every still-referencing Workspace directly to the latest coalesced source state.
    pub fn propagate_source(
        &self,
        selection_key: &SkillSelectionKey,
        updated_at: i64,
    ) -> Result<Vec<(WorkspaceId, Generation)>, DatabaseError> {
        let active = self.pool.with_connection(|connection| {
            load_active_source(connection, selection_key)?.ok_or_else(|| {
                DatabaseError::CorruptEffectState("propagation source is unavailable".to_string())
            })
        })?;
        let affected = self
            .pool
            .with_connection(|connection| referenced_workspaces(connection, selection_key))?;
        let mut advanced = Vec::new();
        for workspace_id in affected {
            let result = self.pool.with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let version = source_version(&active)?;
                let changed = transaction.execute(
                    "UPDATE workspace_effect_desired_skills
                     SET display_name = ?5, source_version = ?6, skill_md_digest = ?7
                     WHERE workspace_id = ?1 AND source_kind = ?2 AND namespace = ?3
                       AND skill_name = ?4
                       AND (source_version <> ?6 OR skill_md_digest <> ?7)",
                    params![
                        workspace_id.as_ref(),
                        source_kind_value(selection_key.source_kind),
                        selection_key.namespace.as_ref(),
                        selection_key.name.canonical(),
                        active.state().name.as_str(),
                        version.as_str(),
                        active.state().skill_md_digest.as_str(),
                    ],
                )?;
                if changed == 0 {
                    transaction.commit()?;
                    return Ok(None);
                }
                let generation = current_generation(&transaction, &workspace_id)?
                    .next()
                    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
                transaction.execute(
                    "UPDATE workspace_effects SET generation = ?2, updated_at = ?3
                     WHERE workspace_id = ?1",
                    params![
                        workspace_id.as_ref(),
                        generation_to_sql(generation)?,
                        updated_at
                    ],
                )?;
                enqueue_workspace_surfaces(&transaction, &workspace_id, generation, updated_at)?;
                transaction.commit()?;
                Ok(Some(generation))
            })?;
            if let Some(generation) = result {
                advanced.push((workspace_id, generation));
            }
        }
        let active_version = source_version(&active)?;
        self.pool.with_connection(|connection| {
            connection.execute(
                "DELETE FROM effect_source_propagation_requests
                 WHERE source_kind = ?1 AND namespace = ?2 AND skill_name = ?3
                   AND requested_version = ?4",
                params![
                    source_kind_value(selection_key.source_kind),
                    selection_key.namespace.as_ref(),
                    selection_key.name.canonical(),
                    active_version.as_str(),
                ],
            )?;
            Ok(())
        })?;
        Ok(advanced)
    }

    /// Lists coalesced source wakeups in deterministic order for an explicitly driven worker.
    pub fn list_propagation_requests(&self) -> Result<Vec<SkillSelectionKey>, DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT source_kind, namespace, skill_name
                 FROM effect_source_propagation_requests
                 ORDER BY requested_at, source_kind, namespace, skill_name",
            )?;
            let mut rows = statement.query([])?;
            let mut requests = Vec::new();
            while let Some(row) = rows.next()? {
                requests.push(map_selection_key(row)?);
            }
            Ok(requests)
        })
    }
}

impl EffectRepository for SqliteEffectRepository {
    fn load_workspace_effect(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceEffect, RepositoryError> {
        self.pool
            .with_connection(|connection| load_workspace_effect(connection, workspace_id))
            .map_err(effect_repository_error)
    }

    fn replace_workspace_effect(
        &self,
        workspace_id: &WorkspaceId,
        expected_generation: Generation,
        spec: WorkspaceEffectSpec,
        updated_at: i64,
    ) -> Result<ReplaceEffectOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let current = load_workspace_effect(&transaction, workspace_id)?;
                if current.generation != expected_generation {
                    transaction.commit()?;
                    return Ok(ReplaceEffectOutcome::Conflict {
                        expected_generation,
                        current_generation: current.generation,
                    });
                }
                for (selection_key, desired) in &spec.skills {
                    let active = load_active_source(&transaction, selection_key)?;
                    if active.as_ref() != Some(desired) {
                        transaction.commit()?;
                        return Ok(ReplaceEffectOutcome::SourceUnavailable {
                            selection_key: selection_key.clone(),
                        });
                    }
                }
                if current.spec == spec {
                    transaction.commit()?;
                    return Ok(ReplaceEffectOutcome::Unchanged(current));
                }
                let generation = current
                    .generation
                    .next()
                    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
                transaction.execute(
                    "INSERT INTO workspace_effects (workspace_id, generation, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(workspace_id) DO UPDATE SET
                         generation = excluded.generation, updated_at = excluded.updated_at",
                    params![
                        workspace_id.as_ref(),
                        generation_to_sql(generation)?,
                        updated_at
                    ],
                )?;
                transaction.execute(
                    "DELETE FROM workspace_effect_desired_skills WHERE workspace_id = ?1",
                    params![workspace_id.as_ref()],
                )?;
                for (selection_key, desired) in &spec.skills {
                    insert_desired(&transaction, workspace_id, selection_key, desired)?;
                }
                enqueue_workspace_surfaces(&transaction, workspace_id, generation, updated_at)?;
                transaction.execute(
                    "INSERT INTO effect_audit_events (
                         workspace_id, event_kind, generation, occurred_at
                     ) VALUES (?1, 'desired_replaced', ?2, ?3)",
                    params![
                        workspace_id.as_ref(),
                        generation_to_sql(generation)?,
                        updated_at
                    ],
                )?;
                transaction.commit()?;
                Ok(ReplaceEffectOutcome::Replaced(WorkspaceEffect {
                    workspace_id: workspace_id.clone(),
                    generation,
                    spec,
                }))
            })
            .map_err(effect_repository_error)
    }

    fn load_managed_skills(
        &self,
        workspace_id: &WorkspaceId,
        surface_key: &ora_effect::SurfaceKey,
    ) -> Result<Vec<ManagedSkill>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT managed_identity, workspace_id, surface_key, source_kind, namespace,
                            skill_name, display_name, source_version, skill_md_digest, locator,
                            target_name, applied_fingerprint, applied_generation
                     FROM effect_managed_skills
                     WHERE workspace_id = ?1 AND surface_key = ?2
                     ORDER BY locator, managed_identity",
                )?;
                let mut rows =
                    statement.query(params![workspace_id.as_ref(), surface_key.as_str()])?;
                let mut managed = Vec::new();
                while let Some(row) = rows.next()? {
                    managed.push(map_managed(row)?);
                }
                Ok(managed)
            })
            .map_err(effect_repository_error)
    }

    fn save_managed_skill(&self, managed: ManagedSkill) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| save_managed(connection, &managed))
            .map_err(effect_repository_error)
    }

    fn delete_managed_skill(
        &self,
        managed_identity: &ManagedIdentity,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM effect_managed_skills WHERE managed_identity = ?1",
                    params![managed_identity.as_str()],
                )?;
                Ok(())
            })
            .map_err(effect_repository_error)
    }

    fn load_surface_status(
        &self,
        workspace_id: &WorkspaceId,
        surface_key: &ora_effect::SurfaceKey,
    ) -> Result<Option<SurfaceStatus>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT workspace_id, surface_key, desired_generation,
                                observed_generation, applied_generation, phase, revision,
                                updated_at, conditions_json
                         FROM effect_surface_status
                         WHERE workspace_id = ?1 AND surface_key = ?2",
                        params![workspace_id.as_ref(), surface_key.as_str()],
                        map_surface_status,
                    )
                    .optional()
                    .map_err(Into::into)
            })
            .map_err(effect_repository_error)
    }

    fn save_surface_status(&self, status: SurfaceStatus) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let conditions =
                    serde_json::to_string(&status.conditions).map_err(effect_json_error)?;
                connection.execute(
                    "INSERT INTO effect_surface_status (
                         workspace_id, surface_key, desired_generation, observed_generation,
                         applied_generation, phase, revision, updated_at, conditions_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(workspace_id, surface_key) DO UPDATE SET
                         desired_generation = excluded.desired_generation,
                         observed_generation = excluded.observed_generation,
                         applied_generation = excluded.applied_generation,
                         phase = excluded.phase, revision = excluded.revision,
                         updated_at = excluded.updated_at,
                         conditions_json = excluded.conditions_json",
                    params![
                        status.workspace_id.as_ref(),
                        status.surface_key.as_str(),
                        generation_to_sql(status.desired_generation)?,
                        generation_to_sql(status.observed_generation)?,
                        generation_to_sql(status.applied_generation)?,
                        surface_phase_value(status.phase),
                        u64_to_sql(status.revision, "status revision")?,
                        status.updated_at,
                        conditions,
                    ],
                )?;
                Ok(())
            })
            .map_err(effect_repository_error)
    }

    fn prepare_operation(&self, operation: EffectOperation) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| insert_operation(connection, &operation))
            .map_err(effect_repository_error)
    }

    fn save_operation(&self, operation: EffectOperation) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| update_operation(connection, &operation))
            .map_err(effect_repository_error)
    }

    fn finalize_operation(
        &self,
        operation: EffectOperation,
        transition: LedgerTransition,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                match transition {
                    LedgerTransition::Upsert(managed) => save_managed(&transaction, &managed)?,
                    LedgerTransition::Replace {
                        previous_identity,
                        next,
                    } => {
                        transaction.execute(
                            "DELETE FROM effect_managed_skills WHERE managed_identity = ?1",
                            params![previous_identity.as_str()],
                        )?;
                        save_managed(&transaction, &next)?;
                    }
                    LedgerTransition::Delete { managed_identity } => {
                        transaction.execute(
                            "DELETE FROM effect_managed_skills WHERE managed_identity = ?1",
                            params![managed_identity.as_str()],
                        )?;
                    }
                }
                update_operation(&transaction, &operation)?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(effect_repository_error)
    }

    fn load_unfinished_operations(&self) -> Result<Vec<EffectOperation>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT payload_json FROM effect_operations
                     WHERE phase <> 'finalized'
                     ORDER BY prepared_at, operation_id",
                )?;
                let mut rows = statement.query([])?;
                let mut operations = Vec::new();
                while let Some(row) = rows.next()? {
                    let payload: String = row.get(0)?;
                    operations.push(serde_json::from_str(&payload).map_err(effect_json_error)?);
                }
                Ok(operations)
            })
            .map_err(effect_repository_error)
    }

    fn save_consumer_status(&self, status: ConsumerStatus) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let conditions =
                    serde_json::to_string(&status.conditions).map_err(effect_json_error)?;
                connection.execute(
                    "INSERT INTO effect_consumer_status (
                         surface_key, consumer_id, ready_generation, phase, revision,
                         updated_at, conditions_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(surface_key, consumer_id) DO UPDATE SET
                         ready_generation = excluded.ready_generation,
                         phase = excluded.phase, revision = excluded.revision,
                         updated_at = excluded.updated_at,
                         conditions_json = excluded.conditions_json",
                    params![
                        status.surface_key.as_str(),
                        status.consumer_id.as_str(),
                        generation_to_sql(status.ready_generation)?,
                        surface_phase_value(status.phase),
                        u64_to_sql(status.revision, "consumer revision")?,
                        status.updated_at,
                        conditions,
                    ],
                )?;
                Ok(())
            })
            .map_err(effect_repository_error)
    }

    fn retry_surface(
        &self,
        workspace_id: &WorkspaceId,
        surface_key: &ora_effect::SurfaceKey,
        requested_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let exists = transaction.query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM effect_surfaces
                         WHERE workspace_id = ?1 AND surface_key = ?2
                     )",
                    params![workspace_id.as_ref(), surface_key.as_str()],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                if exists {
                    upsert_reconcile_request(
                        &transaction,
                        workspace_id,
                        surface_key.as_str(),
                        current_generation(&transaction, workspace_id)?,
                        requested_at,
                    )?;
                }
                transaction.commit()?;
                Ok(exists)
            })
            .map_err(effect_repository_error)
    }
}

impl SourceProvider for SqliteEffectRepository {
    fn open_snapshot(&self, desired: &DesiredSkillState) -> Result<SourceSnapshot, SourceError> {
        let selection_key = source_selection(desired).map_err(source_provider_error)?;
        let loaded = self
            .pool
            .with_connection(|connection| {
                let active = load_active_source(connection, &selection_key)?;
                let package_root = connection
                    .query_row(
                        "SELECT package_root FROM effect_source_states
                         WHERE source_kind = ?1 AND namespace = ?2 AND skill_name = ?3
                           AND availability = 'available'",
                        params![
                            source_kind_value(selection_key.source_kind),
                            selection_key.namespace.as_ref(),
                            selection_key.name.canonical(),
                        ],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                Ok(active.zip(package_root))
            })
            .map_err(source_provider_error)?;
        let Some((active, package_root)) = loaded else {
            return Err(SourceError::Unavailable);
        };
        if &active != desired {
            return Err(SourceError::Unavailable);
        }
        let snapshot = SourceSnapshot::copy_from(active, Path::new(&package_root))?;
        let manifest = fs::read(snapshot.package_root.join("SKILL.md")).map_err(|source| {
            SourceError::Provider {
                source: Box::new(source),
            }
        })?;
        let parsed = ora_skill_package::parse_manifest(
            &manifest,
            ora_skill_package::Limits::default().max_manifest_bytes,
        )
        .map_err(|_| SourceError::IntegrityMismatch)?;
        if parsed.name != desired.state().name.as_str()
            || Digest::sha256(&manifest) != desired.state().skill_md_digest
        {
            return Err(SourceError::IntegrityMismatch);
        }
        Ok(snapshot)
    }

    fn load_active_state(
        &self,
        selection_key: &SkillSelectionKey,
    ) -> Result<DesiredSkillState, SourceError> {
        self.pool
            .with_connection(|connection| load_active_source(connection, selection_key))
            .map_err(source_provider_error)?
            .ok_or(SourceError::Unavailable)
    }

    fn verify_version(
        &self,
        selection_key: &SkillSelectionKey,
        version: &SourceVersion,
    ) -> Result<(), SourceError> {
        let active = self.load_active_state(selection_key)?;
        if source_version(&active).map_err(source_provider_error)? == version {
            Ok(())
        } else {
            Err(SourceError::IntegrityMismatch)
        }
    }
}
