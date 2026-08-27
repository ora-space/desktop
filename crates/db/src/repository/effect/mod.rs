mod mapping;

use crate::{DatabaseError, RepositoryPool};
use mapping::*;
use ora_domain::WorkspaceId;
use ora_effect::{
    ConsumerCoordination, ConsumerId, ConsumerStatus, DesiredSkillState, Digest, EffectOperation,
    EffectRepository, Generation, LedgerTransition, ManagedIdentity, ManagedSkill,
    MaterializationFormat, ReplaceEffectOutcome, RepositoryError, SkillSelectionKey, SourceError,
    SourceProvider, SourceSnapshot, SourceVersion, SurfaceDescriptorSet, SurfaceKey,
    SurfaceLifecycle, SurfacePath, SurfaceStatus, WorkspaceEffect, WorkspaceEffectSpec,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
        if key.source_kind == ora_effect::SourceKind::Local
            && key.namespace != ora_domain::Namespace::local()
        {
            return Err(DatabaseError::CorruptEffectState(
                "Local Skill sources must use the `local` namespace".to_string(),
            ));
        }
        let version = source_version(source)?;
        let payload = serde_json::json!({
            "display_name": source.state().name.as_str(),
            "package_root": package_root.to_string_lossy(),
        })
        .to_string();
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let existing_source_id = transaction
                .query_row(
                    "SELECT id FROM effect_sources
                     WHERE effect_kind = 'skill' AND source_kind = ?1
                       AND namespace = ?2 AND identifier = ?3",
                    params![
                        source_kind_value(key.source_kind),
                        key.namespace.as_ref(),
                        key.name.canonical(),
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let source_id = existing_source_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            transaction.execute(
                "INSERT INTO effect_sources (
                     id, effect_kind, source_kind, namespace, identifier, lifecycle,
                     created_at, updated_at
                 ) VALUES (?1, 'skill', ?2, ?3, ?4, 'active', ?5, ?5)
                 ON CONFLICT(effect_kind, source_kind, namespace, identifier) DO UPDATE SET
                     lifecycle = 'active', updated_at = excluded.updated_at",
                params![
                    &source_id,
                    source_kind_value(key.source_kind),
                    key.namespace.as_ref(),
                    key.name.canonical(),
                    updated_at,
                ],
            )?;
            let existing_revision = transaction
                .query_row(
                    "SELECT id, state_digest, payload_json FROM effect_source_revisions
                     WHERE source_id = ?1 AND revision = ?2",
                    params![&source_id, version.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            let revision_id = if let Some((revision_id, digest, stored_payload)) = existing_revision
            {
                if digest != source.state().skill_md_digest.as_str() || stored_payload != payload {
                    return Err(DatabaseError::CorruptEffectState(
                        "an immutable source revision was republished with different content"
                            .to_string(),
                    ));
                }
                transaction.execute(
                    "UPDATE effect_source_revisions
                     SET availability = 'available', unavailable_reason = NULL, updated_at = ?2
                     WHERE id = ?1",
                    params![&revision_id, updated_at],
                )?;
                revision_id
            } else {
                let revision_id = Uuid::new_v4().to_string();
                transaction.execute(
                    "INSERT INTO effect_source_revisions (
                         id, source_id, revision, state_digest, payload_json, availability,
                         unavailable_reason, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 'available', NULL, ?6, ?6)",
                    params![
                        &revision_id,
                        &source_id,
                        version.as_str(),
                        source.state().skill_md_digest.as_str(),
                        &payload,
                        updated_at,
                    ],
                )?;
                revision_id
            };
            let previous_head = transaction
                .query_row(
                    "SELECT revision_id FROM effect_source_heads WHERE source_id = ?1",
                    params![&source_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            transaction.execute(
                "INSERT INTO effect_source_heads (source_id, revision_id, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(source_id) DO UPDATE SET
                     revision_id = excluded.revision_id, updated_at = excluded.updated_at",
                params![&source_id, &revision_id, updated_at],
            )?;
            if existing_source_id.is_none() {
                install_source_in_all_workspaces(
                    &transaction,
                    &source_id,
                    &revision_id,
                    updated_at,
                )?;
            }
            if publication == SourcePublication::Update
                && previous_head.as_deref() != Some(revision_id.as_str())
            {
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
                "UPDATE effect_source_revisions
                 SET availability = 'unavailable', unavailable_reason = ?4, updated_at = ?5
                 WHERE id = (
                     SELECT heads.revision_id
                     FROM effect_sources sources
                     JOIN effect_source_heads heads ON heads.source_id = sources.id
                     WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?1
                       AND sources.namespace = ?2 AND sources.identifier = ?3
                 )",
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

    /// Retires a source and removes it from every Workspace in one immediate transaction.
    pub fn delete_source(
        &self,
        selection_key: &SkillSelectionKey,
    ) -> Result<SourceMutationOutcome, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let source_id = transaction
                .query_row(
                    "SELECT id FROM effect_sources
                     WHERE effect_kind = 'skill' AND source_kind = ?1
                       AND namespace = ?2 AND identifier = ?3",
                    params![
                        source_kind_value(selection_key.source_kind),
                        selection_key.namespace.as_ref(),
                        selection_key.name.canonical(),
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(source_id) = source_id else {
                transaction.commit()?;
                return Ok(SourceMutationOutcome::Missing);
            };
            uninstall_source_from_all_workspaces(&transaction, &source_id, 0)?;
            transaction.execute(
                "UPDATE effect_sources SET lifecycle = 'retired'
                 WHERE id = ?1",
                params![&source_id],
            )?;
            transaction.execute(
                "UPDATE effect_source_revisions
                 SET availability = 'unavailable', unavailable_reason = 'source was removed'
                 WHERE source_id = ?1",
                params![&source_id],
            )?;
            transaction.execute(
                "DELETE FROM effect_propagation_requests WHERE source_id = ?1",
                params![&source_id],
            )?;
            transaction.commit()?;
            Ok(SourceMutationOutcome::Deleted)
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
                    "SELECT id FROM effect_surfaces
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
                         WHERE workspace_id = ?1 AND id = ?2",
                        params![workspace_id.as_ref(), surface_key, updated_at],
                    )?;
                    upsert_reconcile_request(
                        &transaction,
                        workspace_id,
                        &surface_key,
                        generation,
                        updated_at,
                        "surface_retiring",
                    )?;
                }
            }
            for surface in surfaces {
                let locator_json = serde_json::json!({
                    "workspace_root": workspace_path.to_string_lossy(),
                    "relative_path": surface.path.as_str(),
                })
                .to_string();
                transaction.execute(
                    "INSERT INTO effect_surfaces (
                         id, workspace_id, adapter_kind, locator_key, locator_json,
                         format_kind, lifecycle, created_at, updated_at
                     ) VALUES (?1, ?2, 'filesystem_directory', ?3, ?4, ?5, 'active', ?6, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         lifecycle = 'active', updated_at = excluded.updated_at",
                    params![
                        surface.surface_key.as_str(),
                        workspace_id.as_ref(),
                        surface.path.as_str(),
                        locator_json,
                        surface.format.as_str(),
                        updated_at,
                    ],
                )?;
                let mut consumer_statement = transaction.prepare(
                    "SELECT consumer_id FROM effect_surface_consumers WHERE surface_id = ?1",
                )?;
                let existing_consumers = consumer_statement
                    .query_map(params![surface.surface_key.as_str()], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(consumer_statement);
                for existing_consumer in existing_consumers {
                    if !surface
                        .consumers
                        .keys()
                        .any(|consumer| consumer.as_str() == existing_consumer)
                    {
                        transaction.execute(
                            "DELETE FROM effect_surface_consumers
                             WHERE surface_id = ?1 AND consumer_id = ?2",
                            params![surface.surface_key.as_str(), existing_consumer],
                        )?;
                    }
                }
                for (consumer, coordination) in &surface.consumers {
                    let coordination = match coordination {
                        ora_effect::ConsumerCoordination::Uninterrupted => "uninterrupted",
                        ora_effect::ConsumerCoordination::WaitForIdleAndRestart => {
                            "wait_for_idle_and_restart"
                        }
                    };
                    transaction.execute(
                        "INSERT INTO effect_surface_consumers (
                             surface_id, consumer_id, coordination_kind, created_at, updated_at
                         ) VALUES (?1, ?2, ?3, ?4, ?4)
                         ON CONFLICT(surface_id, consumer_id) DO UPDATE SET
                             coordination_kind = excluded.coordination_kind,
                             updated_at = excluded.updated_at",
                        params![
                            surface.surface_key.as_str(),
                            consumer.as_str(),
                            coordination,
                            updated_at,
                        ],
                    )?;
                    transaction.execute(
                        "INSERT INTO effect_consumer_status (
                             surface_id, consumer_id, ready_generation, phase,
                             status_version, created_at, updated_at
                         ) VALUES (?1, ?2, 0, 'pending', 1, ?3, ?3)
                         ON CONFLICT(surface_id, consumer_id) DO NOTHING",
                        params![surface.surface_key.as_str(), consumer.as_str(), updated_at],
                    )?;
                }
                transaction.execute(
                    "INSERT INTO effect_surface_status (
                         surface_id, desired_generation, observed_generation, applied_generation,
                         phase, status_version, created_at, updated_at
                     ) VALUES (?1, ?2, 0, 0, 'pending', 1, ?3, ?3)
                     ON CONFLICT(surface_id) DO UPDATE SET
                         desired_generation = MAX(desired_generation, excluded.desired_generation),
                         status_version = status_version + 1, updated_at = excluded.updated_at",
                    params![
                        surface.surface_key.as_str(),
                        generation_to_sql(generation)?,
                        updated_at,
                    ],
                )?;
                upsert_reconcile_request(
                    &transaction,
                    workspace_id,
                    surface.surface_key.as_str(),
                    generation,
                    updated_at,
                    "surface_declared",
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    /// Lists Workspaces that already own at least one active surface.
    ///
    /// Surface registration is otherwise driven by a consumer's declaration, which can only see
    /// the Workspaces that existed when it ran. Convergence needs exactly this set to tell a
    /// Workspace that was already offered a declaration apart from one that never was.
    pub fn list_workspaces_with_active_surfaces(
        &self,
    ) -> Result<BTreeSet<WorkspaceId>, DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT workspace_id FROM effect_surfaces WHERE lifecycle = 'active'",
            )?;
            let registered = statement
                .query_map([], |row| Ok(WorkspaceId::new(row.get::<_, String>(0)?)))?
                .collect::<Result<BTreeSet<_>, _>>()?;
            Ok(registered)
        })
    }

    /// Advances every still-referencing Workspace directly to the latest coalesced source state.
    pub fn propagate_source(
        &self,
        selection_key: &SkillSelectionKey,
        updated_at: i64,
    ) -> Result<Vec<(WorkspaceId, Generation)>, DatabaseError> {
        self.pool.with_connection(|connection| {
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
                let changed = transaction.execute(
                    "UPDATE workspace_effect_desired_items
                     SET revision_id = (
                         SELECT heads.revision_id
                         FROM effect_sources sources
                         JOIN effect_source_heads heads ON heads.source_id = sources.id
                         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?2
                           AND sources.namespace = ?3 AND sources.identifier = ?4
                     ), updated_at = ?5
                     WHERE workspace_id = ?1 AND source_id = (
                         SELECT id FROM effect_sources
                         WHERE effect_kind = 'skill' AND source_kind = ?2
                           AND namespace = ?3 AND identifier = ?4
                     ) AND revision_id <> (
                         SELECT heads.revision_id
                         FROM effect_sources sources
                         JOIN effect_source_heads heads ON heads.source_id = sources.id
                         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?2
                           AND sources.namespace = ?3 AND sources.identifier = ?4
                     )",
                    params![
                        workspace_id.as_ref(),
                        source_kind_value(selection_key.source_kind),
                        selection_key.namespace.as_ref(),
                        selection_key.name.canonical(),
                        updated_at,
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
        self.pool.with_connection(|connection| {
            connection.execute(
                "DELETE FROM effect_propagation_requests
                 WHERE source_id = (
                     SELECT id FROM effect_sources
                     WHERE effect_kind = 'skill' AND source_kind = ?1
                       AND namespace = ?2 AND identifier = ?3
                 ) AND head_revision_id = (
                     SELECT heads.revision_id
                     FROM effect_sources sources
                     JOIN effect_source_heads heads ON heads.source_id = sources.id
                     WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?1
                       AND sources.namespace = ?2 AND sources.identifier = ?3
                 )",
                params![
                    source_kind_value(selection_key.source_kind),
                    selection_key.namespace.as_ref(),
                    selection_key.name.canonical(),
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
                "SELECT sources.source_kind, sources.namespace, sources.identifier AS skill_name
                 FROM effect_propagation_requests requests
                 JOIN effect_sources sources ON sources.id = requests.source_id
                 ORDER BY requests.requested_at, sources.source_kind,
                          sources.namespace, sources.identifier",
            )?;
            let mut rows = statement.query([])?;
            let mut requests = Vec::new();
            while let Some(row) = rows.next()? {
                requests.push(map_selection_key(row)?);
            }
            Ok(requests)
        })
    }

    /// Atomically takes ownership of the surfaces currently owed a reconcile.
    ///
    /// The rows are durable level-triggered state, not an event log: a worker reads what is owed
    /// now rather than replaying how it came to be owed, which is what lets repeated wakeups merge
    /// into one request. Claiming inside a single immediate transaction is what makes two workers
    /// unable to hold the same surface, and a claim whose lease has expired is reclaimable so a
    /// crashed worker cannot strand a surface forever.
    ///
    /// The returned token fences every later transition: a worker that lost its lease while it was
    /// stalled finds its token replaced and can no longer write the outcome of stale work.
    pub fn claim_due_reconcile_requests(
        &self,
        worker_id: &str,
        now: i64,
        lease_expires_at: i64,
        limit: usize,
    ) -> Result<Vec<ClaimedReconcile>, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let claimable = {
                let mut statement = transaction.prepare(
                    "SELECT surface_id FROM effect_reconcile_requests
                     WHERE (state IN ('pending', 'retry_scheduled') AND not_before_at <= ?1)
                        OR (state = 'claimed' AND lease_expires_at <= ?1)
                     ORDER BY not_before_at, requested_at, surface_id
                     LIMIT ?2",
                )?;
                let mut rows = statement.query(params![now, limit as i64])?;
                let mut keys = Vec::new();
                while let Some(row) = rows.next()? {
                    keys.push(row.get::<_, String>(0)?);
                }
                keys
            };

            let mut claimed = Vec::new();
            for surface_key in claimable {
                let token = Uuid::new_v4().to_string();
                let taken = transaction.execute(
                    "UPDATE effect_reconcile_requests
                     SET state = 'claimed', lease_owner = ?2, lease_expires_at = ?3,
                         request_token = ?4, blocked_reason = NULL,
                         attempt_count = attempt_count + 1, updated_at = ?5
                     WHERE surface_id = ?1
                       AND ((state IN ('pending', 'retry_scheduled') AND not_before_at <= ?5)
                            OR (state = 'claimed' AND lease_expires_at <= ?5))",
                    params![&surface_key, worker_id, lease_expires_at, &token, now],
                )?;
                if taken == 0 {
                    continue;
                }
                let (mapped, attempt) = transaction.query_row(
                    "SELECT surfaces.id, surfaces.workspace_id, surfaces.locator_key,
                            surfaces.locator_json, surfaces.format_kind, surfaces.lifecycle,
                            requests.requested_generation, requests.attempt_count
                     FROM effect_reconcile_requests requests
                     JOIN effect_surfaces surfaces ON surfaces.id = requests.surface_id
                     WHERE requests.surface_id = ?1",
                    params![&surface_key],
                    |row| Ok((map_due_reconcile(row), row.get::<_, i64>(7)?)),
                )?;
                let mut due = mapped?;
                {
                    let mut statement = transaction.prepare(
                        "SELECT consumer_id, coordination_kind FROM effect_surface_consumers
                         WHERE surface_id = ?1 ORDER BY consumer_id",
                    )?;
                    let mut rows = statement.query(params![&surface_key])?;
                    while let Some(row) = rows.next()? {
                        let consumer = ConsumerId::new(row.get::<_, String>(0)?);
                        let coordination = parse_coordination(&row.get::<_, String>(1)?)?;
                        due.descriptor.consumers.insert(consumer, coordination);
                    }
                }
                claimed.push(ClaimedReconcile {
                    claim: ReconcileClaim {
                        surface_key: due.descriptor.surface_key.clone(),
                        token,
                        attempt,
                    },
                    due,
                });
            }
            transaction.commit()?;
            Ok(claimed)
        })
    }

    /// Extends a claim that is still genuinely held, reporting whether it survived.
    ///
    /// A `false` answer means the lease was taken over while this worker was busy. That is the one
    /// moment a worker must stop touching the surface: another worker is already reconciling it,
    /// and continuing would apply two plans to the same targets.
    pub fn renew_reconcile_claim(
        &self,
        claim: &ReconcileClaim,
        worker_id: &str,
        lease_expires_at: i64,
        now: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let renewed = connection.execute(
                "UPDATE effect_reconcile_requests
                 SET lease_expires_at = ?3, updated_at = ?4
                 WHERE surface_id = ?1 AND request_token = ?2 AND state = 'claimed'
                   AND lease_owner = ?5",
                params![
                    claim.surface_key.as_str(),
                    &claim.token,
                    lease_expires_at,
                    now,
                    worker_id,
                ],
            )?;
            Ok(renewed > 0)
        })
    }

    /// Clears the request when the reconcile reached the generation it was asked for.
    ///
    /// A Desired edit landing mid-reconcile raises `requested_generation`, so an unconditional
    /// delete would drop a wakeup nothing would re-create. Falling back to `pending` instead of
    /// deleting is what lets the worker loop straight onto the newer generation.
    pub fn complete_reconcile_request(
        &self,
        claim: &ReconcileClaim,
        completed_generation: Generation,
        now: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let removed = transaction.execute(
                "DELETE FROM effect_reconcile_requests
                 WHERE surface_id = ?1 AND request_token = ?2
                   AND requested_generation <= ?3",
                params![
                    claim.surface_key.as_str(),
                    &claim.token,
                    generation_to_sql(completed_generation)?,
                ],
            )?;
            if removed == 0 {
                transaction.execute(
                    "UPDATE effect_reconcile_requests
                     SET state = 'pending', lease_owner = NULL, lease_expires_at = NULL,
                         blocked_reason = NULL, attempt_count = 0,
                         not_before_at = MAX(requested_at, ?3), updated_at = ?3
                     WHERE surface_id = ?1 AND request_token = ?2 AND state = 'claimed'",
                    params![claim.surface_key.as_str(), &claim.token, now],
                )?;
            }
            transaction.commit()?;
            Ok(removed > 0)
        })
    }

    /// Parks a request until an external fact changes, without scheduling a timed retry.
    ///
    /// A surface waiting on an idle Session, a missing consumer, or an unresolved conflict cannot
    /// be helped by trying again sooner, so it is left owed but not runnable. The safety scan and
    /// the next Desired or declaration change are what re-arm it.
    pub fn block_reconcile_request(
        &self,
        claim: &ReconcileClaim,
        reason: &str,
        now: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let blocked = connection.execute(
                "UPDATE effect_reconcile_requests
                 SET state = 'blocked', blocked_reason = ?3, lease_owner = NULL,
                     lease_expires_at = NULL, updated_at = ?4
                 WHERE surface_id = ?1 AND request_token = ?2 AND state = 'claimed'",
                params![claim.surface_key.as_str(), &claim.token, reason, now],
            )?;
            Ok(blocked > 0)
        })
    }

    /// Schedules the next attempt after a transient failure, persisting the backoff itself.
    ///
    /// The delay lives in the row rather than in the worker, so a restart cannot turn a backing-off
    /// surface back into an immediate retry and spin on the same failure.
    pub fn retry_reconcile_request(
        &self,
        claim: &ReconcileClaim,
        reason: &str,
        not_before_at: i64,
        now: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let scheduled = connection.execute(
                "UPDATE effect_reconcile_requests
                 SET state = 'retry_scheduled', blocked_reason = NULL, wake_reason = ?3,
                     lease_owner = NULL, lease_expires_at = NULL,
                     not_before_at = MAX(requested_at, ?4), updated_at = ?5
                 WHERE surface_id = ?1 AND request_token = ?2 AND state = 'claimed'",
                params![
                    claim.surface_key.as_str(),
                    &claim.token,
                    reason,
                    not_before_at,
                    now,
                ],
            )?;
            Ok(scheduled > 0)
        })
    }

    /// Rebuilds durable work that a crash, a lost notification, or drift left unscheduled.
    ///
    /// Correctness cannot rest on a process staying alive, so this recreates a request for every
    /// surface whose durable state still proves work is owed: a generation short of applied, an
    /// operation left unfinished, or a retirement that never completed. Requests already claimed by
    /// a live lease are left alone.
    pub fn recover_reconcile_requests(&self, now: i64) -> Result<usize, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            // A lease that outlived its worker is released rather than reclaimed here, so the
            // ordinary claim path decides who picks it up next.
            let released = transaction.execute(
                "UPDATE effect_reconcile_requests
                 SET state = 'pending', lease_owner = NULL, lease_expires_at = NULL,
                     not_before_at = MAX(requested_at, ?1), updated_at = ?1
                 WHERE state = 'claimed' AND lease_expires_at <= ?1",
                params![now],
            )?;
            let unconverged = transaction.execute(
                "INSERT INTO effect_reconcile_requests (
                     surface_id, requested_generation, request_token, state, wake_reason,
                     blocked_reason, attempt_count, requested_at, not_before_at, updated_at
                 )
                 SELECT status.surface_id, status.desired_generation, ?2, 'pending',
                        'startup_recovery', NULL, 0, ?1, ?1, ?1
                 FROM effect_surface_status status
                 JOIN effect_surfaces surfaces ON surfaces.id = status.surface_id
                 WHERE (status.applied_generation < status.desired_generation
                        OR surfaces.lifecycle = 'retiring'
                        OR EXISTS (
                            SELECT 1 FROM effect_operations operations
                            WHERE operations.surface_id = status.surface_id
                              AND operations.phase != 'finalized'
                        ))
                   AND NOT EXISTS (
                       SELECT 1 FROM effect_reconcile_requests existing
                       WHERE existing.surface_id = status.surface_id
                   )",
                params![now, Uuid::new_v4().to_string()],
            )?;
            transaction.commit()?;
            Ok(released + unconverged)
        })
    }

    /// Re-arms blocked requests so a lost runtime event cannot park a surface permanently.
    ///
    /// Blocking is correct — nothing is gained by retrying a surface waiting on a busy Session —
    /// but the event that should unblock it travels in process and can be lost to a crash. This is
    /// the low-frequency safety net for that, not the primary schedule.
    pub fn rearm_blocked_reconcile_requests(&self, now: i64) -> Result<usize, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let rearmed = connection.execute(
                "UPDATE effect_reconcile_requests
                 SET state = 'pending', blocked_reason = NULL, wake_reason = 'safety_scan',
                     not_before_at = MAX(requested_at, ?1), updated_at = ?1
                 WHERE state = 'blocked'",
                params![now],
            )?;
            Ok(rearmed)
        })
    }

    /// Ends a retired surface's lifecycle once its ownership ledger is provably empty.
    ///
    /// Deletion is refused while any Managed item still names the surface: that ledger is the only
    /// proof of what Ora may still touch on disk, so it has to outlive the declaration that created
    /// it instead of disappearing through a cascade.
    pub fn delete_retired_surface(&self, surface_key: &SurfaceKey) -> Result<bool, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let managed: i64 = transaction.query_row(
                "SELECT count(*) FROM effect_managed_items WHERE surface_id = ?1",
                params![surface_key.as_str()],
                |row| row.get(0),
            )?;
            if managed > 0 {
                transaction.commit()?;
                return Ok(false);
            }
            let deleted = transaction.execute(
                "DELETE FROM effect_surfaces WHERE id = ?1 AND lifecycle = 'retiring'",
                params![surface_key.as_str()],
            )?;
            transaction.commit()?;
            Ok(deleted > 0)
        })
    }
}

/// One surface owing a reconcile, with the Workspace root its adapter must be rooted at.
#[derive(Clone, Debug)]
pub struct DueSurfaceReconcile {
    pub workspace_id: WorkspaceId,
    pub workspace_root: PathBuf,
    pub descriptor: SurfaceDescriptorSet,
    pub requested_generation: Generation,
}

/// Proof that one worker currently owns a surface, and the fence its writes are checked against.
///
/// The token is regenerated on every claim, so a worker whose lease expired and was taken over
/// cannot commit the outcome of work the new owner has already redone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileClaim {
    pub surface_key: SurfaceKey,
    pub token: String,
    /// How many attempts this request has cost, including the one just claimed; drives backoff.
    pub attempt: i64,
}

/// A claimed request together with everything the reconciler needs to serve it.
#[derive(Clone, Debug)]
pub struct ClaimedReconcile {
    pub claim: ReconcileClaim,
    pub due: DueSurfaceReconcile,
}

/// Rebuilds one descriptor from its persisted locator, leaving consumers for the caller to attach.
fn map_due_reconcile(row: &rusqlite::Row<'_>) -> Result<DueSurfaceReconcile, DatabaseError> {
    let locator: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>(3)?).map_err(effect_json_error)?;
    let workspace_root = locator
        .get("workspace_root")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            DatabaseError::CorruptEffectState(
                "Effect surface locator is missing workspace_root".to_string(),
            )
        })?;
    let lifecycle = match row.get::<_, String>(5)?.as_str() {
        "active" => SurfaceLifecycle::Active,
        "retiring" => SurfaceLifecycle::Retiring,
        other => {
            return Err(DatabaseError::CorruptEffectState(format!(
                "unknown Effect surface lifecycle {other}"
            )));
        }
    };
    Ok(DueSurfaceReconcile {
        workspace_id: WorkspaceId::new(row.get::<_, String>(1)?),
        workspace_root: PathBuf::from(workspace_root),
        descriptor: SurfaceDescriptorSet {
            surface_key: SurfaceKey::new(row.get::<_, String>(0)?),
            path: SurfacePath::parse(&row.get::<_, String>(2)?).map_err(|error| {
                DatabaseError::CorruptEffectState(format!("invalid Effect surface path: {error}"))
            })?,
            format: MaterializationFormat::named(row.get::<_, String>(4)?).map_err(|error| {
                DatabaseError::CorruptEffectState(format!("invalid Effect surface format: {error}"))
            })?,
            consumers: BTreeMap::new(),
            lifecycle,
        },
        requested_generation: generation_from_sql(row.get::<_, i64>(6)?)?,
    })
}

/// Maps the persisted coordination policy back onto its domain value.
fn parse_coordination(value: &str) -> Result<ConsumerCoordination, DatabaseError> {
    match value {
        "uninterrupted" => Ok(ConsumerCoordination::Uninterrupted),
        "wait_for_idle_and_restart" => Ok(ConsumerCoordination::WaitForIdleAndRestart),
        other => Err(DatabaseError::CorruptEffectState(format!(
            "unknown Effect consumer coordination {other}"
        ))),
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
                    "INSERT INTO workspace_effects (
                         workspace_id, generation, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?3)
                     ON CONFLICT(workspace_id) DO UPDATE SET
                         generation = excluded.generation, updated_at = excluded.updated_at",
                    params![
                        workspace_id.as_ref(),
                        generation_to_sql(generation)?,
                        updated_at
                    ],
                )?;
                transaction.execute(
                    "DELETE FROM workspace_effect_desired_items WHERE workspace_id = ?1",
                    params![workspace_id.as_ref()],
                )?;
                for (selection_key, desired) in &spec.skills {
                    insert_desired(
                        &transaction,
                        workspace_id,
                        selection_key,
                        desired,
                        updated_at,
                    )?;
                }
                enqueue_workspace_surfaces(&transaction, workspace_id, generation, updated_at)?;
                transaction.execute(
                    "INSERT INTO effect_audit_events (
                         id, workspace_id, subject_kind, subject_id, event_kind, generation,
                         initiator_kind, payload_version, payload_json, occurred_at
                     ) VALUES (?1, ?2, 'workspace_effect', ?2, 'desired_replaced', ?3,
                               'user', 1, '{}', ?4)",
                    params![
                        Uuid::new_v4().to_string(),
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
                    "SELECT managed.id, surfaces.workspace_id, managed.surface_id,
                            sources.source_kind, sources.namespace,
                            sources.identifier AS skill_name,
                            json_extract(revisions.payload_json, '$.display_name') AS display_name,
                            revisions.revision AS source_version,
                            revisions.state_digest AS skill_md_digest,
                            managed.target_key, managed.target_json,
                            managed.applied_fingerprint, managed.applied_generation
                     FROM effect_managed_items managed
                     JOIN effect_surfaces surfaces ON surfaces.id = managed.surface_id
                     JOIN effect_sources sources ON sources.id = managed.source_id
                     JOIN effect_source_revisions revisions
                       ON revisions.id = managed.applied_revision_id
                     WHERE surfaces.workspace_id = ?1 AND managed.surface_id = ?2
                     ORDER BY managed.target_key, managed.id",
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
                    "DELETE FROM effect_managed_items WHERE id = ?1",
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
                let mut status = connection
                    .query_row(
                        "SELECT surfaces.workspace_id, status.surface_id AS surface_key,
                                status.desired_generation, status.observed_generation,
                                status.applied_generation, status.phase,
                                status.status_version AS revision, status.updated_at,
                                '[]' AS conditions_json
                         FROM effect_surface_status status
                         JOIN effect_surfaces surfaces ON surfaces.id = status.surface_id
                         WHERE surfaces.workspace_id = ?1 AND status.surface_id = ?2",
                        params![workspace_id.as_ref(), surface_key.as_str()],
                        map_surface_status,
                    )
                    .optional()?;
                if let Some(status) = &mut status {
                    status.conditions = load_conditions(
                        connection,
                        surface_key.as_str(),
                        /*consumer_id*/ None,
                    )?;
                }
                Ok(status)
            })
            .map_err(effect_repository_error)
    }

    fn save_surface_status(&self, status: SurfaceStatus) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute(
                    "INSERT INTO effect_surface_status (
                         surface_id, desired_generation, observed_generation,
                         applied_generation, phase, status_version, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                     ON CONFLICT(surface_id) DO UPDATE SET
                         desired_generation = excluded.desired_generation,
                         observed_generation = excluded.observed_generation,
                         applied_generation = excluded.applied_generation,
                         phase = excluded.phase, status_version = excluded.status_version,
                         updated_at = excluded.updated_at",
                    params![
                        status.surface_key.as_str(),
                        generation_to_sql(status.desired_generation)?,
                        generation_to_sql(status.observed_generation)?,
                        generation_to_sql(status.applied_generation)?,
                        surface_phase_value(status.phase),
                        u64_to_sql(status.revision, "status revision")?,
                        status.updated_at,
                    ],
                )?;
                replace_conditions(
                    &transaction,
                    status.surface_key.as_str(),
                    /*consumer_id*/ None,
                    &status.conditions,
                )?;
                transaction.commit()?;
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
                            "DELETE FROM effect_managed_items WHERE id = ?1",
                            params![previous_identity.as_str()],
                        )?;
                        save_managed(&transaction, &next)?;
                    }
                    LedgerTransition::Delete { managed_identity } => {
                        transaction.execute(
                            "DELETE FROM effect_managed_items WHERE id = ?1",
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
                     ORDER BY prepared_at, id",
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

    fn complete_operation_cleanup(
        &self,
        operation_id: &ora_effect::EffectOperationId,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM effect_operation_artifacts
                     WHERE operation_id = ?1 AND state = 'pending_cleanup'",
                    params![operation_id.as_str()],
                )?;
                Ok(())
            })
            .map_err(effect_repository_error)
    }

    fn save_consumer_status(&self, status: ConsumerStatus) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute(
                    "INSERT INTO effect_consumer_status (
                         surface_id, consumer_id, ready_generation, phase, status_version,
                         created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                     ON CONFLICT(surface_id, consumer_id) DO UPDATE SET
                         ready_generation = excluded.ready_generation,
                         phase = excluded.phase, status_version = excluded.status_version,
                         updated_at = excluded.updated_at",
                    params![
                        status.surface_key.as_str(),
                        status.consumer_id.as_str(),
                        generation_to_sql(status.ready_generation)?,
                        surface_phase_value(status.phase),
                        u64_to_sql(status.revision, "consumer revision")?,
                        status.updated_at,
                    ],
                )?;
                replace_conditions(
                    &transaction,
                    status.surface_key.as_str(),
                    Some(status.consumer_id.as_str()),
                    &status.conditions,
                )?;
                transaction.commit()?;
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
                         WHERE workspace_id = ?1 AND id = ?2
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
                        "user_retry",
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
                        "SELECT json_extract(revisions.payload_json, '$.package_root')
                         FROM effect_sources sources
                         JOIN effect_source_heads heads ON heads.source_id = sources.id
                         JOIN effect_source_revisions revisions ON revisions.id = heads.revision_id
                         WHERE sources.effect_kind = 'skill' AND sources.source_kind = ?1
                           AND sources.namespace = ?2 AND sources.identifier = ?3
                           AND sources.lifecycle = 'active'
                           AND revisions.availability = 'available'",
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
