use super::SqliteEffectRepository;
use super::mapping::{effect_json, generation_from_sql, generation_to_sql};
use super::source::{advance_changed_scopes, seed_scope_sources};
use crate::DatabaseError;
use ora_domain::{Workspace, WorkspaceLocation};
use ora_effect::{
    ConsumerDeclaration, ConsumerIdentity, ConsumerRevisionId, Digest, EffectResourceId,
    EffectScopeId, EffectTargetId, FilesystemDirectoryDescriptor, FilesystemFileDescriptor,
    LocalTimestamp, ResourceAdapterIdentity, TargetDeclaration, TargetResourceBinding,
    VersionedResourceDescriptor,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};

impl SqliteEffectRepository {
    /// Declares one stable Consumer Revision and pairs it with every eligible existing Workspace.
    pub fn declare_consumer(
        &self,
        declaration: &ConsumerDeclaration,
        workspaces: &[Workspace],
        updated_at: LocalTimestamp,
    ) -> Result<ConsumerRevisionId, DatabaseError> {
        declaration
            .validate()
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let (consumer_id, revision_id, revision_changed) =
                upsert_consumer_revision(&transaction, declaration, updated_at.millis())?;
            let mut changed_scopes = BTreeSet::new();
            for workspace in workspaces {
                let WorkspaceLocation::LocalFilesystem { path } = &workspace.location else {
                    continue;
                };
                let scope = EffectScopeId::Workspace(workspace.id.clone());
                seed_scope_sources(
                    &transaction,
                    &scope,
                    updated_at.millis(),
                    &mut changed_scopes,
                )?;
                upsert_workspace_target(
                    &transaction,
                    &scope,
                    path,
                    &consumer_id,
                    &revision_id,
                    declaration,
                    revision_changed,
                    updated_at.millis(),
                )?;
            }
            advance_changed_scopes(&transaction, &changed_scopes, updated_at.millis())?;
            transaction.commit()?;
            Ok(revision_id)
        })
    }

    /// Marks a Consumer and all of its Targets Retiring while retaining cleanup bindings.
    pub fn retire_consumer(
        &self,
        consumer: &ConsumerIdentity,
        updated_at: LocalTimestamp,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let consumer_id = consumer.storage_key();
            let changed = transaction.execute(
                "UPDATE effect_consumers SET lifecycle = 'retiring', updated_at = ?2
                 WHERE id = ?1 AND lifecycle = 'declared'",
                params![&consumer_id, updated_at.millis()],
            )?;
            if changed == 0 {
                transaction.commit()?;
                return Ok(false);
            }
            let mut statement = transaction.prepare(
                "SELECT target.id, target.scope_id, scope.generation
                 FROM effect_targets target
                 JOIN effect_scopes scope ON scope.id = target.scope_id
                 WHERE target.consumer_id = ?1 AND target.lifecycle = 'active'",
            )?;
            let targets = statement
                .query_map(params![&consumer_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for (target_id, scope_id, generation) in targets {
                transaction.execute(
                    "UPDATE effect_targets SET lifecycle = 'retiring', updated_at = ?2
                     WHERE id = ?1",
                    params![&target_id, updated_at.millis()],
                )?;
                transaction.execute(
                    "UPDATE effect_target_status
                     SET phase = 'retiring', status_version = status_version + 1, updated_at = ?2
                     WHERE target_id = ?1",
                    params![&target_id, updated_at.millis()],
                )?;
                upsert_target_wakeup(
                    &transaction,
                    &target_id,
                    generation_from_sql(generation)?,
                    updated_at.millis(),
                    "target_retiring",
                )?;
                let _ = scope_id;
            }
            transaction.commit()?;
            Ok(true)
        })
    }
}

/// Creates a new immutable Consumer Revision only when the complete declaration changes.
fn upsert_consumer_revision(
    transaction: &Transaction<'_>,
    declaration: &ConsumerDeclaration,
    updated_at: i64,
) -> Result<(String, ConsumerRevisionId, bool), DatabaseError> {
    let consumer_id = declaration.consumer.storage_key();
    let declaration_json = effect_json(declaration)?;
    let declaration_digest = Digest::sha256(declaration_json.as_bytes());
    let current = transaction
        .query_row(
            "SELECT consumer.current_revision_id, revision.declaration_digest
             FROM effect_consumers consumer
             JOIN effect_consumer_revisions revision
               ON revision.id = consumer.current_revision_id
             WHERE consumer.id = ?1",
            params![&consumer_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((revision_id, digest)) = current
        && digest == declaration_digest.as_str()
    {
        transaction.execute(
            "UPDATE effect_consumers
             SET lifecycle = 'declared', adapter_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![&consumer_id, declaration.adapter.as_str(), updated_at,],
        )?;
        return Ok((consumer_id, ConsumerRevisionId::new(revision_id), false));
    }

    let revision_id = ConsumerRevisionId::random();
    transaction.execute(
        "INSERT INTO effect_consumers (
             id, consumer_kind, identity_key, adapter_id, current_revision_id,
             lifecycle, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'declared', ?6, ?6)
         ON CONFLICT(id) DO UPDATE SET adapter_id = excluded.adapter_id,
             current_revision_id = excluded.current_revision_id,
             lifecycle = 'declared', updated_at = excluded.updated_at",
        params![
            &consumer_id,
            declaration.consumer.kind.as_str(),
            &declaration.consumer.stable_key,
            declaration.adapter.as_str(),
            revision_id.as_str(),
            updated_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO effect_consumer_revisions (
             id, consumer_id, capability_version, capabilities_json,
             declaration_digest, created_at
         ) VALUES (?1, ?2, 1, ?3, ?4, ?5)",
        params![
            revision_id.as_str(),
            &consumer_id,
            effect_json(&declaration.capabilities)?,
            declaration_digest.as_str(),
            updated_at,
        ],
    )?;
    Ok((consumer_id, revision_id, true))
}

/// Creates or replaces one `(Scope, Consumer)` active Target from the immutable declaration.
#[allow(clippy::too_many_arguments)]
fn upsert_workspace_target(
    transaction: &Transaction<'_>,
    scope: &EffectScopeId,
    workspace_path: &str,
    consumer_id: &str,
    revision_id: &ConsumerRevisionId,
    declaration: &ConsumerDeclaration,
    revision_changed: bool,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    let scope_id = scope.storage_key();
    let generation = generation_from_sql(transaction.query_row(
        "SELECT generation FROM effect_scopes WHERE id = ?1 AND lifecycle = 'active'",
        params![&scope_id],
        |row| row.get::<_, i64>(0),
    )?)?;
    let active_target = transaction
        .query_row(
            "SELECT target.id, declaration.consumer_revision_id
             FROM effect_targets target
             JOIN effect_target_declarations declaration ON declaration.target_id = target.id
             WHERE target.scope_id = ?1 AND target.consumer_id = ?2
               AND target.lifecycle = 'active'",
            params![&scope_id, consumer_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let target_id = match active_target {
        Some((target_id, current_revision))
            if !revision_changed && current_revision == revision_id.as_str() =>
        {
            // An immutable declaration digest identifies the complete Target shape, so touching
            // status or wakeup rows here would create an endless level-triggered reconcile loop.
            let _ = target_id;
            return Ok(());
        }
        Some((target_id, _)) => {
            retire_target(transaction, &target_id, generation, updated_at)?;
            EffectTargetId::random()
        }
        None => EffectTargetId::random(),
    };

    transaction.execute(
        "INSERT INTO effect_targets (
             id, scope_id, consumer_id, consumer_revision_id, lifecycle, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET consumer_revision_id = excluded.consumer_revision_id,
             updated_at = excluded.updated_at",
        params![
            target_id.as_str(),
            &scope_id,
            consumer_id,
            revision_id.as_str(),
            updated_at,
        ],
    )?;

    let mut bindings = BTreeMap::new();
    for template in &declaration.resources {
        let resource_id = upsert_resource(
            transaction,
            scope,
            workspace_path,
            template,
            generation,
            updated_at,
        )?;
        let binding = TargetResourceBinding {
            target: target_id.clone(),
            resource: resource_id.clone(),
            materialization_contract: template.materialization_contract.clone(),
            accepts: template.accepts.clone(),
            coordination: template.coordination.clone(),
        };
        insert_binding(transaction, &scope_id, &binding)?;
        bindings.insert(resource_id, binding);
    }
    let target_declaration = TargetDeclaration {
        target: target_id.clone(),
        consumer_revision: revision_id.clone(),
        bindings,
        digest: Digest::sha256(effect_json(&declaration.resources)?.as_bytes()),
    };
    transaction.execute(
        "INSERT INTO effect_target_declarations (
             target_id, consumer_revision_id, declaration_version, declaration_json,
             digest, updated_at
         ) VALUES (?1, ?2, 1, ?3, ?4, ?5)
         ON CONFLICT(target_id) DO UPDATE SET
             consumer_revision_id = excluded.consumer_revision_id,
             declaration_json = excluded.declaration_json, digest = excluded.digest,
             updated_at = excluded.updated_at",
        params![
            target_id.as_str(),
            revision_id.as_str(),
            effect_json(&target_declaration)?,
            target_declaration.digest.as_str(),
            updated_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO effect_target_status (
             target_id, desired_generation, observed_generation, applied_generation,
             ready_generation, phase, recovery_operation_id, status_version,
             created_at, updated_at
         ) VALUES (?1, ?2, 0, 0, 0, 'pending', NULL, 1, ?3, ?3)
         ON CONFLICT(target_id) DO UPDATE SET
             desired_generation = MAX(desired_generation, excluded.desired_generation),
             phase = CASE WHEN phase = 'recovery_required' THEN phase ELSE 'pending' END,
             status_version = status_version + 1, updated_at = excluded.updated_at",
        params![
            target_id.as_str(),
            generation_to_sql(generation)?,
            updated_at
        ],
    )?;
    upsert_target_wakeup(
        transaction,
        target_id.as_str(),
        generation,
        updated_at,
        "declaration_changed",
    )?;
    Ok(())
}

/// Finds or creates the single active Resource for one normalized physical key in a Scope.
fn upsert_resource(
    transaction: &Transaction<'_>,
    scope: &EffectScopeId,
    workspace_path: &str,
    template: &ora_effect::FilesystemResourceTemplate,
    generation: ora_effect::Generation,
    updated_at: i64,
) -> Result<EffectResourceId, DatabaseError> {
    let scope_id = scope.storage_key();
    let resource_key = template.resource_key();
    let descriptor = match &template.ownership_relative_path {
        Some(ownership_relative_path) => {
            VersionedResourceDescriptor::FilesystemFileV1(FilesystemFileDescriptor {
                workspace_root: std::path::PathBuf::from(workspace_path),
                relative_path: template.relative_path.clone(),
                ownership_relative_path: ownership_relative_path.clone(),
            })
        }
        None => VersionedResourceDescriptor::FilesystemDirectoryV1(FilesystemDirectoryDescriptor {
            workspace_root: std::path::PathBuf::from(workspace_path),
            relative_path: template.relative_path.clone(),
        }),
    };
    let descriptor_json = effect_json(&descriptor)?;
    let existing = transaction
        .query_row(
            "SELECT id, descriptor_json, materialization_format
             FROM effect_resources
             WHERE scope_id = ?1 AND resource_key = ?2 AND lifecycle = 'active'",
            params![&scope_id, resource_key.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let resource_id = match existing {
        Some((id, stored_descriptor, stored_format)) => {
            if stored_descriptor != descriptor_json
                || stored_format != template.materialization_format.as_str()
            {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "incompatible declarations for shared Resource {resource_key}"
                )));
            }
            EffectResourceId::new(id)
        }
        None => {
            let id = EffectResourceId::random();
            let adapter_name = if template.ownership_relative_path.is_some() {
                "ora/json-file-merge"
            } else {
                "ora/filesystem-directory"
            };
            let adapter = ResourceAdapterIdentity::parse(adapter_name)
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
            transaction.execute(
                "INSERT INTO effect_resources (
                     id, scope_id, resource_key, adapter_id, descriptor_version,
                     descriptor_json, materialization_format, lifecycle, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, 'active', ?7, ?7)",
                params![
                    id.as_str(),
                    &scope_id,
                    resource_key.as_str(),
                    adapter.as_str(),
                    &descriptor_json,
                    template.materialization_format.as_str(),
                    updated_at,
                ],
            )?;
            transaction.execute(
                "INSERT INTO effect_resource_status (
                     resource_id, desired_generation, observed_generation, applied_generation,
                     phase, recovery_operation_id, status_version, created_at, updated_at
                 ) VALUES (?1, ?2, 0, 0, 'pending', NULL, 1, ?3, ?3)",
                params![id.as_str(), generation_to_sql(generation)?, updated_at],
            )?;
            id
        }
    };
    Ok(resource_id)
}

/// Persists one typed Target/Resource binding and its optional coordination contract.
fn insert_binding(
    transaction: &Transaction<'_>,
    scope_id: &str,
    binding: &TargetResourceBinding,
) -> Result<(), DatabaseError> {
    let (coordination_kind, contract_version, contract_json) = match &binding.coordination {
        ora_effect::CoordinationRequirement::Uninterrupted => ("uninterrupted", None, None),
        ora_effect::CoordinationRequirement::QuiesceBeforeMutation(contract) => (
            "quiesce_before_mutation",
            Some(i64::from(contract.version)),
            Some(effect_json(contract)?),
        ),
    };
    transaction.execute(
        "INSERT INTO effect_target_resource_bindings (
             scope_id, target_id, resource_id, accepts_version, accepts_json,
             coordination_kind, coordination_contract_version, coordination_contract_json
         ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7)
         ON CONFLICT(target_id, resource_id) DO UPDATE SET
             accepts_json = excluded.accepts_json,
             coordination_kind = excluded.coordination_kind,
             coordination_contract_version = excluded.coordination_contract_version,
             coordination_contract_json = excluded.coordination_contract_json",
        params![
            scope_id,
            binding.target.as_str(),
            binding.resource.as_str(),
            effect_json(&binding.accepts)?,
            coordination_kind,
            contract_version,
            contract_json,
        ],
    )?;
    Ok(())
}

/// Retires the old complete Target snapshot so its bindings remain available for cleanup.
fn retire_target(
    transaction: &Transaction<'_>,
    target_id: &str,
    generation: ora_effect::Generation,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "UPDATE effect_targets SET lifecycle = 'retiring', updated_at = ?2 WHERE id = ?1",
        params![target_id, updated_at],
    )?;
    transaction.execute(
        "UPDATE effect_target_status
         SET phase = 'retiring', status_version = status_version + 1, updated_at = ?2
         WHERE target_id = ?1",
        params![target_id, updated_at],
    )?;
    upsert_target_wakeup(
        transaction,
        target_id,
        generation,
        updated_at,
        "target_retiring",
    )
}

/// Coalesces a direct Target wakeup with the same claimed-request preservation as Scope wakeups.
pub(super) fn upsert_target_wakeup(
    transaction: &Connection,
    target_id: &str,
    generation: ora_effect::Generation,
    updated_at: i64,
    reason: &str,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_reconcile_requests (
             target_id, requested_generation, state, wake_reasons_json, retry_count,
             claim_token, claim_worker, lease_until, retry_attempt, not_before,
             blocked_conditions_json, resume_trigger_version, resume_trigger_json,
             requested_at, updated_at
         ) VALUES (?1, ?2, 'pending', json_array(?4), 0,
                   NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?3, ?3)
         ON CONFLICT(target_id) DO UPDATE SET
             requested_generation = MAX(requested_generation, excluded.requested_generation),
             state = CASE
                 WHEN state = 'claimed' THEN 'claimed'
                 WHEN state = 'blocked' AND EXISTS (
                     SELECT 1 FROM effect_reconcile_attempts attempt
                     JOIN effect_operations operation ON operation.attempt_id = attempt.id
                     WHERE attempt.target_id = excluded.target_id
                       AND operation.phase = 'recovery_required'
                 ) THEN 'blocked'
                 ELSE 'pending'
             END,
             wake_reasons_json = excluded.wake_reasons_json,
             retry_count = 0,
             claim_token = CASE WHEN state = 'claimed' THEN claim_token ELSE NULL END,
             claim_worker = CASE WHEN state = 'claimed' THEN claim_worker ELSE NULL END,
             lease_until = CASE WHEN state = 'claimed' THEN lease_until ELSE NULL END,
             retry_attempt = NULL, not_before = NULL,
             blocked_conditions_json = CASE
                 WHEN state = 'blocked' AND EXISTS (
                     SELECT 1 FROM effect_reconcile_attempts attempt
                     JOIN effect_operations operation ON operation.attempt_id = attempt.id
                     WHERE attempt.target_id = excluded.target_id
                       AND operation.phase = 'recovery_required'
                 ) THEN blocked_conditions_json ELSE NULL
             END,
             resume_trigger_version = CASE
                 WHEN state = 'blocked' AND EXISTS (
                     SELECT 1 FROM effect_reconcile_attempts attempt
                     JOIN effect_operations operation ON operation.attempt_id = attempt.id
                     WHERE attempt.target_id = excluded.target_id
                       AND operation.phase = 'recovery_required'
                 ) THEN resume_trigger_version ELSE NULL
             END,
             resume_trigger_json = CASE
                 WHEN state = 'blocked' AND EXISTS (
                     SELECT 1 FROM effect_reconcile_attempts attempt
                     JOIN effect_operations operation ON operation.attempt_id = attempt.id
                     WHERE attempt.target_id = excluded.target_id
                       AND operation.phase = 'recovery_required'
                 ) THEN resume_trigger_json ELSE NULL
             END,
             requested_at = excluded.requested_at, updated_at = excluded.updated_at",
        params![
            target_id,
            generation_to_sql(generation)?,
            updated_at,
            reason,
        ],
    )?;
    Ok(())
}
