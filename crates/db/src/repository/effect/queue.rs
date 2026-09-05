use super::EffectWriteContext;
use super::SqliteEffectRepository;
use super::claims::{
    claim_is_valid, load_managed, load_related_target_ids, release_resource_claims, verify_claim,
    verify_resource_claims,
};
use super::conditions::replace_conditions;
use super::ledger_validation::validate_projection_managed_transition;
use super::mapping::{
    effect_json, generation_from_sql, load_consumer_revision, load_desired_state, load_resource,
    load_resource_status, load_revisions_for_scope, load_target, load_target_declaration,
    load_target_status,
};
use super::persistence::{
    complete_request, finish_retiring_target, group_conditions, insert_readiness,
    save_current_state,
};
use super::projection_persistence::save_projections;
use super::store::{save_resource_status, save_target_status};
use super::validation::{validate_current_state_scope, validate_projection_scope};
use crate::DatabaseError;
use crate::TimestampSource;
use ora_effect::{
    ConditionOwner, ConditionProposal, EffectResourceId, EffectTargetId, FencingToken,
    LocalTimestamp, ProjectionCommit, ReconcileClaim, ReconcileRequest, ReconcileRequestState,
    ReconcileSnapshot, RelatedTargetSnapshot, ResourceClaim, ResourceStatus, WorkerIdentity,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};

/// Claims due Target requests under a monotonically increasing fencing token.
pub(super) fn claim_due_targets<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    worker: &WorkerIdentity,
    now: LocalTimestamp,
    lease_until: LocalTimestamp,
    limit: usize,
) -> Result<Vec<(EffectTargetId, ReconcileClaim)>, DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        let limit = i64::try_from(limit).map_err(|_| {
            DatabaseError::CorruptEffectState(
                "Effect claim limit exceeds SQLite INTEGER".to_string(),
            )
        })?;
        let target_ids = {
            let mut statement = transaction.prepare(
                "SELECT target_id FROM effect_reconcile_requests
                 WHERE (
                     (state = 'pending')
                     OR (state = 'retry_scheduled' AND not_before <= ?1)
                     OR (state = 'claimed' AND lease_until <= ?1)
                 )
                 AND NOT EXISTS (
                     SELECT 1 FROM effect_reconcile_attempts attempt
                     JOIN effect_operations operation ON operation.attempt_id = attempt.id
                     WHERE attempt.target_id = effect_reconcile_requests.target_id
                       AND operation.phase <> 'finalized'
                 )
                 ORDER BY COALESCE(not_before, requested_at), requested_at, target_id
                 LIMIT ?2",
            )?;
            statement
                .query_map(params![now.millis(), limit], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut claimed = Vec::new();
        for target_id in target_ids {
            let fence_updated = transaction.execute(
                "UPDATE effect_targets SET claim_fence = claim_fence + 1, updated_at = MAX(updated_at, ?2)
                 WHERE id = ?1",
                params![&target_id, written_at.millis()],
            )?;
            if fence_updated == 0 {
                continue;
            }
            let token = transaction.query_row(
                "SELECT claim_fence FROM effect_targets WHERE id = ?1",
                params![&target_id],
                |row| row.get::<_, i64>(0),
            )?;
            let updated = transaction.execute(
                "UPDATE effect_reconcile_requests
                 SET state = 'claimed', claim_token = ?2, claim_worker = ?3, lease_until = ?4,
                     retry_attempt = NULL, not_before = NULL, blocked_conditions_json = NULL,
                     resume_trigger_version = NULL, resume_trigger_json = NULL, updated_at = MAX(updated_at, ?5)
                 WHERE target_id = ?1 AND (
                     state = 'pending'
                     OR (state = 'retry_scheduled' AND not_before <= ?6)
                     OR (state = 'claimed' AND lease_until <= ?6)
                 )",
                params![
                    &target_id,
                    token,
                    worker.as_str(),
                    lease_until.millis(),
                    written_at.millis(),
                    now.millis(),
                ],
            )?;
            if updated == 0 {
                continue;
            }
            claimed.push((
                EffectTargetId::new(target_id),
                ReconcileClaim {
                    token: FencingToken::new(u64::try_from(token).map_err(|_| {
                        DatabaseError::CorruptEffectState(
                            "Effect fencing token is negative".to_string(),
                        )
                    })?),
                    worker: worker.clone(),
                    lease_until,
                },
            ));
        }
        transaction.commit()?;
        Ok(claimed)
    })
}

/// Reloads all Target, shared Resource, capability, ownership, and status facts after claim.
pub(super) fn load_reconcile_snapshot<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    target_id: &EffectTargetId,
    claim: &ReconcileClaim,
) -> Result<ReconcileSnapshot, DatabaseError> {
    repository.pool.with_connection(|connection| {
        verify_claim(connection, target_id, claim)?;
        let target = load_target(connection, target_id)?;
        let desired = load_desired_state(connection, &target.scope)?;
        let consumer_revision = load_consumer_revision(connection, &target.consumer_revision)?;
        let declaration = load_target_declaration(connection, target_id)?;
        let direct_resources = declaration
            .bindings
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        let related_target_ids = load_related_target_ids(connection, &direct_resources)?;
        let mut related_targets = BTreeMap::new();
        let mut participant_targets = BTreeMap::new();
        let mut resources = BTreeMap::new();
        let mut coordination_participants = direct_resources
            .iter()
            .cloned()
            .map(|resource| (resource, BTreeMap::new()))
            .collect::<BTreeMap<_, _>>();
        for related_id in related_target_ids {
            let related_target = load_target(connection, &related_id)?;
            let related_revision =
                load_consumer_revision(connection, &related_target.consumer_revision)?;
            let related_declaration = load_target_declaration(connection, &related_id)?;
            for (resource_id, binding) in &related_declaration.bindings {
                resources
                    .entry(resource_id.clone())
                    .or_insert(load_resource(connection, resource_id)?);
                if let Some(participants) = coordination_participants.get_mut(resource_id) {
                    participants.insert(related_id.clone(), binding.coordination.clone());
                }
            }
            participant_targets.insert(related_id.clone(), related_target.clone());
            related_targets.insert(
                related_id,
                RelatedTargetSnapshot {
                    target: related_target,
                    consumer_revision: related_revision,
                    declaration: related_declaration,
                },
            );
        }
        if !related_targets.contains_key(target_id) {
            related_targets.insert(
                target_id.clone(),
                RelatedTargetSnapshot {
                    target: target.clone(),
                    consumer_revision: consumer_revision.clone(),
                    declaration: declaration.clone(),
                },
            );
            participant_targets.insert(target_id.clone(), target.clone());
        }
        for resource_id in &direct_resources {
            resources
                .entry(resource_id.clone())
                .or_insert(load_resource(connection, resource_id)?);
        }
        let mut resource_statuses = BTreeMap::new();
        let mut managed = BTreeMap::new();
        for resource_id in &direct_resources {
            let status = load_resource_status(connection, resource_id)?.ok_or_else(|| {
                DatabaseError::CorruptEffectState(format!(
                    "missing Effect Resource status {resource_id}"
                ))
            })?;
            resource_statuses.insert(resource_id.clone(), status);
            managed.insert(resource_id.clone(), load_managed(connection, resource_id)?);
        }
        let target_status = load_target_status(connection, target_id)?.ok_or_else(|| {
            DatabaseError::CorruptEffectState(format!("missing Effect Target status {target_id}"))
        })?;
        let requested_generation = connection.query_row(
            "SELECT requested_generation FROM effect_reconcile_requests WHERE target_id = ?1",
            params![target_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        let revisions = load_revisions_for_scope(connection, &target.scope)?;
        Ok(ReconcileSnapshot {
            request: ReconcileRequest {
                target: target_id.clone(),
                requested_generation: generation_from_sql(requested_generation)?,
                state: ReconcileRequestState::Claimed(claim.clone()),
                wake_reasons: BTreeSet::new(),
            },
            claim: claim.clone(),
            desired,
            target,
            consumer_revision,
            declaration,
            resources,
            revisions,
            related_targets,
            coordination_participants,
            participant_targets,
            target_status,
            resource_statuses,
            managed,
        })
    })
}

/// Acquires independently fenced Resource leases in stable identity order or none at all.
pub(super) fn claim_resources<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    target: &EffectTargetId,
    claim: &ReconcileClaim,
    resources: &[EffectResourceId],
    now: LocalTimestamp,
    lease_until: LocalTimestamp,
) -> Result<Option<Vec<ResourceClaim>>, DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        verify_claim(&transaction, target, claim)?;
        let scope_id = transaction.query_row(
            "SELECT scope_id FROM effect_targets WHERE id = ?1",
            params![target.as_str()],
            |row| row.get::<_, String>(0),
        )?;
        let mut ordered = resources.to_vec();
        ordered.sort();
        ordered.dedup();
        let mut acquired = Vec::new();
        for resource in ordered {
            let current = transaction
                .query_row(
                    "SELECT target_claim_token, resource_fence, worker, lease_until
                     FROM effect_resource_claims WHERE resource_id = ?1",
                    params![resource.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((token, _, worker, current_lease)) = &current
                && *current_lease > now.millis()
                && (u64::try_from(*token).ok() != Some(claim.token.value())
                    || worker != claim.worker.as_str())
            {
                transaction.rollback()?;
                return Ok(None);
            }
            let fence_updated = transaction.execute(
                "UPDATE effect_resources SET claim_fence = claim_fence + 1, updated_at = MAX(updated_at, ?2)
                 WHERE id = ?1 AND scope_id = ?3",
                params![resource.as_str(), written_at.millis(), &scope_id],
            )?;
            if fence_updated != 1 {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "Target {target} cannot claim out-of-Scope Resource {resource}"
                )));
            }
            let next_fence = transaction.query_row(
                "SELECT claim_fence FROM effect_resources WHERE id = ?1",
                params![resource.as_str()],
                |row| row.get::<_, i64>(0),
            )?;
            transaction.execute(
                "INSERT INTO effect_resource_claims (
                     resource_id, scope_id, target_id, target_claim_token, resource_fence,
                     worker, lease_until, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(resource_id) DO UPDATE SET
                     scope_id = excluded.scope_id,
                     target_id = excluded.target_id,
                     target_claim_token = excluded.target_claim_token,
                     resource_fence = excluded.resource_fence,
                     worker = excluded.worker, lease_until = excluded.lease_until,
                     updated_at = MAX(effect_resource_claims.updated_at, excluded.updated_at)",
                params![
                    resource.as_str(),
                    &scope_id,
                    target.as_str(),
                    i64::try_from(claim.token.value()).map_err(|_| {
                        DatabaseError::CorruptEffectState(
                            "Target fencing token exceeds SQLite INTEGER".to_string(),
                        )
                    })?,
                    next_fence,
                    claim.worker.as_str(),
                    lease_until.millis(),
                    written_at.millis(),
                ],
            )?;
            acquired.push(ResourceClaim {
                resource,
                target_claim: claim.token,
                resource_fence: FencingToken::new(u64::try_from(next_fence).map_err(|_| {
                    DatabaseError::CorruptEffectState(
                        "Resource fencing token is negative".to_string(),
                    )
                })?),
                worker: claim.worker.clone(),
                lease_until,
            });
        }
        transaction.commit()?;
        Ok(Some(acquired))
    })
}

/// Persists structured blocking facts and releases the Target claim into Blocked state.
pub(super) fn block_target<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    target: &EffectTargetId,
    claim: &ReconcileClaim,
    target_status: ora_effect::TargetStatus,
    resource_statuses: Vec<ResourceStatus>,
    conditions: Vec<ConditionProposal>,
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        verify_claim(&transaction, target, claim)?;
        let status_resources = resource_statuses
            .iter()
            .map(|status| status.resource().clone())
            .collect::<BTreeSet<_>>();
        validate_current_state_scope(
            &transaction,
            target,
            target_status.progress().observed(),
            &status_resources,
            std::slice::from_ref(&target_status),
            &resource_statuses,
            &[],
            &[],
            &conditions,
        )?;
        verify_resource_claims(&transaction, target, claim, &status_resources)?;
        save_target_status(&transaction, &target_status, written_at)?;
        for status in &resource_statuses {
            save_resource_status(&transaction, status, written_at)?;
        }
        let mut condition_ids = Vec::new();
        let grouped = group_conditions(conditions);
        for (owner, proposals) in &grouped {
            condition_ids.extend(replace_conditions(
                &transaction,
                owner,
                proposals,
                written_at,
            )?);
        }
        if !grouped.contains_key(&ConditionOwner::Target(target.clone())) {
            replace_conditions(
                &transaction,
                &ConditionOwner::Target(target.clone()),
                &[],
                written_at,
            )?;
        }
        let condition_json = effect_json(&condition_ids)?;
        transaction.execute(
            "UPDATE effect_reconcile_requests
             SET state = 'blocked', claim_token = NULL, claim_worker = NULL, lease_until = NULL,
                 retry_attempt = NULL, not_before = NULL, blocked_conditions_json = ?3,
                 resume_trigger_version = 1, resume_trigger_json = '{\"kind\":\"condition_change\"}',
                 updated_at = MAX(updated_at, ?4)
             WHERE target_id = ?1 AND state = 'claimed' AND claim_token = ?2",
            params![
                target.as_str(),
                i64::try_from(claim.token.value()).map_err(|_| {
                    DatabaseError::CorruptEffectState(
                        "Target fencing token exceeds SQLite INTEGER".to_string(),
                    )
                })?,
                condition_json,
                written_at.millis(),
            ],
        )?;
        release_resource_claims(&transaction, target, claim)?;
        transaction.commit()?;
        Ok(())
    })
}

/// Atomically commits a projection that required no external mutation.
pub(super) fn commit_projection<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    claim: &ReconcileClaim,
    commit: ProjectionCommit,
) -> Result<(), DatabaseError> {
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        verify_claim(&transaction, commit.target_status.target(), claim)?;
        let generation = commit.target_status.progress().ready();
        let projected_resources = validate_projection_scope(
            &transaction,
            commit.target_status.target(),
            generation,
            &commit.target_projections,
            &commit.resource_projections,
        )?;
        let claimed_projection = commit
            .target_projections
            .iter()
            .find(|projection| projection.target == *commit.target_status.target())
            .ok_or_else(|| {
                DatabaseError::CorruptEffectState(
                    "Effect projection commit lacks its claimed Target projection".to_string(),
                )
            })?;
        if commit.readiness.as_ref().is_none_or(|readiness| {
            readiness.target != *commit.target_status.target()
                || readiness.generation != generation
                || readiness.consumer_revision != claimed_projection.consumer_revision
                || readiness.projection != claimed_projection.digest
        }) {
            return Err(DatabaseError::CorruptEffectState(
                "Effect projection commit lacks exact readiness evidence".to_string(),
            ));
        }
        let status_resources = commit
            .resource_statuses
            .iter()
            .map(|status| status.resource().clone())
            .collect::<BTreeSet<_>>();
        if status_resources != projected_resources
            || commit
                .resource_statuses
                .iter()
                .any(|status| status.applied() != generation)
        {
            return Err(DatabaseError::CorruptEffectState(
                "Effect projection commit Resource statuses are incomplete".to_string(),
            ));
        }
        verify_resource_claims(
            &transaction,
            commit.target_status.target(),
            claim,
            &projected_resources,
        )?;
        validate_projection_managed_transition(
            &transaction,
            &commit.resource_projections,
            &commit.managed,
            &commit.removed_managed,
        )?;
        validate_current_state_scope(
            &transaction,
            commit.target_status.target(),
            generation,
            &projected_resources,
            std::slice::from_ref(&commit.target_status),
            &commit.resource_statuses,
            &commit.managed,
            &commit.removed_managed,
            &commit.conditions,
        )?;
        save_projections(
            &transaction,
            &commit.target_projections,
            &commit.resource_projections,
            written_at,
        )?;
        save_current_state(
            &transaction,
            &commit.managed,
            &commit.removed_managed,
            std::slice::from_ref(&commit.target_status),
            &commit.resource_statuses,
            &commit.conditions,
            written_at,
        )?;
        if let Some(readiness) = &commit.readiness {
            insert_readiness(&transaction, readiness, written_at)?;
        }
        complete_request(
            &transaction,
            commit.target_status.target(),
            claim,
            commit.target_status.progress().ready(),
            written_at,
        )?;
        release_resource_claims(&transaction, commit.target_status.target(), claim)?;
        finish_retiring_target(&transaction, commit.target_status.target())?;
        transaction.commit()?;
        Ok(())
    })
}

/// Releases a failed Target claim into a counted durable retry without losing newer generations.
pub(super) fn schedule_retry<Clock: TimestampSource>(
    repository: &SqliteEffectRepository<Clock>,
    target: &EffectTargetId,
    claim: &ReconcileClaim,
    not_before: LocalTimestamp,
    scheduled_at: LocalTimestamp,
) -> Result<Option<ora_effect::RetryAttempt>, DatabaseError> {
    if not_before < scheduled_at {
        return Err(DatabaseError::CorruptEffectState(
            "Effect retry cannot be scheduled in the past".to_string(),
        ));
    }
    repository.pool.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written_at = EffectWriteContext::new(&transaction, &repository.clock).timestamp();
        if !claim_is_valid(&transaction, target, claim)? {
            transaction.commit()?;
            return Ok(None);
        }
        let retry_count = transaction.query_row(
            "SELECT retry_count FROM effect_reconcile_requests WHERE target_id = ?1",
            params![target.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        let retry_count = retry_count.checked_add(1).ok_or_else(|| {
            DatabaseError::CorruptEffectState("Effect retry count is exhausted".to_string())
        })?;
        let retry = ora_effect::RetryAttempt::new(u32::try_from(retry_count).map_err(|_| {
            DatabaseError::CorruptEffectState(
                "Effect retry count exceeds its domain representation".to_string(),
            )
        })?);
        let request_updated = transaction.execute(
            "UPDATE effect_reconcile_requests
             SET state = 'retry_scheduled', wake_reasons_json = json_array('retry_due'),
                 retry_count = ?3, claim_token = NULL, claim_worker = NULL, lease_until = NULL,
                 retry_attempt = ?3, not_before = ?4, blocked_conditions_json = NULL,
                 resume_trigger_version = NULL, resume_trigger_json = NULL, updated_at = MAX(updated_at, ?5)
             WHERE target_id = ?1 AND state = 'claimed' AND claim_token = ?2",
            params![
                target.as_str(),
                i64::try_from(claim.token.value()).map_err(|_| {
                    DatabaseError::CorruptEffectState(
                        "Target fencing token exceeds SQLite INTEGER".to_string(),
                    )
                })?,
                retry_count,
                not_before.millis(),
                written_at.millis(),
            ],
        )?;
        if request_updated != 1 {
            return Err(DatabaseError::CorruptEffectState(
                "Effect retry lost its fenced Target claim".to_string(),
            ));
        }
        release_resource_claims(&transaction, target, claim)?;
        transaction.commit()?;
        Ok(Some(retry))
    })
}
