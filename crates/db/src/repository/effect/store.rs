use super::SqliteEffectRepository;
use super::mapping::{
    effect_json, generation_to_sql, load_desired_state, load_target_status, parse_effect_json,
    resource_phase_value, status_version_to_sql, target_phase_value,
};
use super::source::wake_scope_targets;
use crate::DatabaseError;
use ora_effect::{
    ConditionGeneration, ConditionImpact, ConditionOwner, ConditionProposal, ConditionRetry,
    ConditionSubject, DesiredEffect, DesiredState, EffectCondition, EffectRepository,
    EffectScopeId, EffectTargetId, Generation, LocalTimestamp, ReplaceDesiredStateOutcome,
    RepositoryError, ResourceStatus, StableConditionCode, TargetStatus,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use uuid::Uuid;

impl EffectRepository for SqliteEffectRepository {
    fn load_desired_state(&self, scope: &EffectScopeId) -> Result<DesiredState, RepositoryError> {
        self.pool
            .with_connection(|connection| load_desired_state(connection, scope))
            .map_err(RepositoryError::new)
    }

    fn replace_desired_state(
        &self,
        scope: &EffectScopeId,
        expected_generation: Generation,
        effects: Vec<DesiredEffect>,
        updated_at: LocalTimestamp,
    ) -> Result<ReplaceDesiredStateOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let current = load_desired_state(&transaction, scope)?;
                if current.generation != expected_generation {
                    transaction.commit()?;
                    return Ok(ReplaceDesiredStateOutcome::Conflict {
                        expected_generation,
                        current_generation: current.generation,
                    });
                }
                let lifecycle = transaction.query_row(
                    "SELECT lifecycle FROM effect_scopes WHERE id = ?1",
                    params![scope.storage_key()],
                    |row| row.get::<_, String>(0),
                )?;
                if lifecycle == "retiring" {
                    transaction.commit()?;
                    return Ok(ReplaceDesiredStateOutcome::ScopeRetiring);
                }
                let normalized =
                    DesiredState::normalized(scope.clone(), current.generation, effects)
                        .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
                for desired in normalized.effects.values() {
                    let revision = transaction
                        .query_row(
                            "SELECT availability, definition_kind
                             FROM effect_revisions WHERE id = ?1",
                            params![desired.revision.as_str()],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                        )
                        .optional()?;
                    let Some((availability, definition_kind)) = revision else {
                        transaction.commit()?;
                        return Ok(ReplaceDesiredStateOutcome::RevisionUnavailable(
                            desired.revision.clone(),
                        ));
                    };
                    if availability != "available"
                        || definition_kind != parameters_kind(&desired.parameters)
                    {
                        transaction.commit()?;
                        return Ok(ReplaceDesiredStateOutcome::RevisionUnavailable(
                            desired.revision.clone(),
                        ));
                    }
                }
                if current.effects == normalized.effects {
                    transaction.commit()?;
                    return Ok(ReplaceDesiredStateOutcome::Unchanged(current));
                }
                let generation = current
                    .generation
                    .next()
                    .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
                transaction.execute(
                    "DELETE FROM effect_desired_effects WHERE scope_id = ?1",
                    params![scope.storage_key()],
                )?;
                for desired in normalized.effects.values() {
                    insert_desired_effect(&transaction, scope, desired, updated_at.millis())?;
                }
                transaction.execute(
                    "UPDATE effect_scopes SET generation = ?2, updated_at = ?3 WHERE id = ?1",
                    params![
                        scope.storage_key(),
                        generation_to_sql(generation)?,
                        updated_at.millis(),
                    ],
                )?;
                wake_scope_targets(
                    &transaction,
                    &scope.storage_key(),
                    generation,
                    updated_at.millis(),
                    "desired_changed",
                )?;
                transaction.execute(
                    "INSERT INTO effect_audit_events (
                         id, scope_id, subject_kind, subject_id, event_kind, generation,
                         initiator_kind, initiator_id, payload_version, payload_json, occurred_at
                     ) VALUES (?1, ?2, 'desired_state', ?2, 'desired_replaced', ?3,
                               'user', NULL, 1, '{}', ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        scope.storage_key(),
                        generation_to_sql(generation)?,
                        updated_at.millis(),
                    ],
                )?;
                transaction.commit()?;
                Ok(ReplaceDesiredStateOutcome::Replaced(DesiredState {
                    scope: scope.clone(),
                    generation,
                    effects: normalized.effects,
                }))
            })
            .map_err(RepositoryError::new)
    }

    fn load_target_status(
        &self,
        target: &EffectTargetId,
    ) -> Result<Option<(TargetStatus, Vec<EffectCondition>)>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let status = load_target_status(connection, target)?;
                status
                    .map(|status| {
                        load_conditions(connection, &ConditionOwner::Target(target.clone()))
                            .map(|conditions| (status, conditions))
                    })
                    .transpose()
            })
            .map_err(RepositoryError::new)
    }

    fn load_consumer_target_status(
        &self,
        scope: &EffectScopeId,
        consumer: &ora_effect::ConsumerIdentity,
    ) -> Result<Option<(TargetStatus, Vec<EffectCondition>)>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let target = connection
                    .query_row(
                        "SELECT id FROM effect_targets
                         WHERE scope_id = ?1 AND consumer_id = ?2 AND lifecycle = 'active'",
                        params![scope.storage_key(), consumer.storage_key()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                    .map(EffectTargetId::new);
                target
                    .map(|target| {
                        let status = load_target_status(connection, &target)?.ok_or_else(|| {
                            DatabaseError::CorruptEffectState(
                                "active Effect Target has no status".to_string(),
                            )
                        })?;
                        load_conditions(connection, &ConditionOwner::Target(target))
                            .map(|conditions| (status, conditions))
                    })
                    .transpose()
            })
            .map_err(RepositoryError::new)
    }

    fn request_reconcile(
        &self,
        target: &EffectTargetId,
        requested_at: LocalTimestamp,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let generation = transaction
                    .query_row(
                        "SELECT scope.generation
                         FROM effect_targets target
                         JOIN effect_scopes scope ON scope.id = target.scope_id
                         WHERE target.id = ?1",
                        params![target.as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                let Some(generation) = generation else {
                    transaction.commit()?;
                    return Ok(false);
                };
                super::declaration::upsert_target_wakeup(
                    &transaction,
                    target.as_str(),
                    super::mapping::generation_from_sql(generation)?,
                    requested_at.millis(),
                    "user_requested",
                )?;
                transaction.commit()?;
                Ok(true)
            })
            .map_err(RepositoryError::new)
    }

    // Durable claim methods stay in `queue`; journal transitions use the same centralized fences.
    fn claim_due_targets(
        &self,
        worker: &ora_effect::WorkerIdentity,
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
        limit: usize,
    ) -> Result<Vec<(EffectTargetId, ora_effect::ReconcileClaim)>, RepositoryError> {
        super::queue::claim_due_targets(self, worker, now, lease_until, limit)
            .map_err(RepositoryError::new)
    }

    fn load_reconcile_snapshot(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
    ) -> Result<ora_effect::ReconcileSnapshot, RepositoryError> {
        super::queue::load_reconcile_snapshot(self, target, claim).map_err(RepositoryError::new)
    }

    fn claim_resources(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        resources: &[ora_effect::EffectResourceId],
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
    ) -> Result<Option<Vec<ora_effect::ResourceClaim>>, RepositoryError> {
        super::queue::claim_resources(self, target, claim, resources, now, lease_until)
            .map_err(RepositoryError::new)
    }

    fn prepare_attempt(
        &self,
        claim: &ora_effect::ReconcileClaim,
        attempt: ora_effect::ReconcileAttempt,
        target_projections: Vec<ora_effect::TargetProjection>,
        resource_projections: Vec<ora_effect::ResourceProjection>,
        operations: Vec<ora_effect::EffectOperation>,
        artifacts: Vec<ora_effect::OperationArtifact>,
    ) -> Result<(), RepositoryError> {
        super::journal::prepare_attempt(
            self,
            claim,
            attempt,
            target_projections,
            resource_projections,
            operations,
            artifacts,
        )
        .map_err(RepositoryError::new)
    }

    fn record_attempt_progress(
        &self,
        claim: &ora_effect::ReconcileClaim,
        attempt: &ora_effect::ReconcileAttempt,
        operations: &[ora_effect::EffectOperation],
        coordination_receipts: &[ora_effect::CoordinationReceipt],
        updated_at: LocalTimestamp,
    ) -> Result<(), RepositoryError> {
        super::journal::record_attempt_progress(
            self,
            claim,
            attempt,
            operations,
            coordination_receipts,
            updated_at,
        )
        .map_err(RepositoryError::new)
    }

    fn block_target(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        target_status: TargetStatus,
        resource_statuses: Vec<ResourceStatus>,
        conditions: Vec<ConditionProposal>,
        updated_at: LocalTimestamp,
    ) -> Result<(), RepositoryError> {
        super::queue::block_target(
            self,
            target,
            claim,
            target_status,
            resource_statuses,
            conditions,
            updated_at,
        )
        .map_err(RepositoryError::new)
    }

    fn commit_projection(
        &self,
        claim: &ora_effect::ReconcileClaim,
        commit: ora_effect::ProjectionCommit,
    ) -> Result<(), RepositoryError> {
        super::queue::commit_projection(self, claim, commit).map_err(RepositoryError::new)
    }

    fn finalize_attempt(
        &self,
        claim: &ora_effect::ReconcileClaim,
        finalization: ora_effect::AttemptFinalization,
    ) -> Result<(), RepositoryError> {
        super::journal::finalize_attempt(self, claim, finalization).map_err(RepositoryError::new)
    }

    fn schedule_retry(
        &self,
        target: &EffectTargetId,
        claim: &ora_effect::ReconcileClaim,
        not_before: LocalTimestamp,
        updated_at: LocalTimestamp,
    ) -> Result<Option<ora_effect::RetryAttempt>, RepositoryError> {
        super::queue::schedule_retry(self, target, claim, not_before, updated_at)
            .map_err(RepositoryError::new)
    }

    fn load_unfinished_operations(
        &self,
    ) -> Result<Vec<ora_effect::EffectOperation>, RepositoryError> {
        super::recovery::load_unfinished_operations(self).map_err(RepositoryError::new)
    }

    fn quarantine_unfinished_operations(
        &self,
        detected_at: LocalTimestamp,
    ) -> Result<usize, RepositoryError> {
        super::recovery::quarantine_unfinished_operations(self, detected_at)
            .map_err(RepositoryError::new)
    }

    fn complete_artifact_cleanup(
        &self,
        artifact: &ora_effect::ArtifactId,
        receipt: ora_effect::CleanupReceipt,
    ) -> Result<(), RepositoryError> {
        super::recovery::complete_artifact_cleanup(self, artifact, receipt)
            .map_err(RepositoryError::new)
    }

    fn mark_artifact_cleanup_failed(
        &self,
        artifact: ora_effect::OperationArtifact,
        failed_at: LocalTimestamp,
    ) -> Result<(), RepositoryError> {
        super::recovery::mark_artifact_cleanup_failed(self, artifact, failed_at)
            .map_err(RepositoryError::new)
    }
}

/// Inserts one fully typed Desired Effect row at a well-defined persistence boundary.
fn insert_desired_effect(
    transaction: &Transaction<'_>,
    scope: &EffectScopeId,
    desired: &DesiredEffect,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_desired_effects (
             id, scope_id, revision_id, parameters_kind, parameters_version,
             parameters_json, selector_version, selector_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 1, ?5, 1, ?6, ?7, ?7)",
        params![
            desired.identity.as_str(),
            scope.storage_key(),
            desired.revision.as_str(),
            parameters_kind(&desired.parameters),
            effect_json(&desired.parameters)?,
            effect_json(&desired.audience)?,
            updated_at,
        ],
    )?;
    Ok(())
}

/// Returns the persisted kind discriminator for typed parameters.
fn parameters_kind(parameters: &ora_effect::ValidatedEffectParameters) -> &'static str {
    match parameters {
        ora_effect::ValidatedEffectParameters::Skill(_) => "skill",
    }
}

/// Replaces current Conditions for one owner while retaining identity and first-observed time.
pub(super) fn replace_conditions(
    transaction: &Transaction<'_>,
    owner: &ConditionOwner,
    proposals: &[ConditionProposal],
    observed_at: LocalTimestamp,
) -> Result<Vec<ora_effect::ConditionId>, DatabaseError> {
    let (owner_kind, owner_id) = owner_parts(owner);
    let mut existing = BTreeMap::new();
    {
        let mut statement = transaction.prepare(
            "SELECT id, subject_kind, subject_id, code, first_observed_at
             FROM effect_conditions WHERE owner_kind = ?1 AND owner_id = ?2",
        )?;
        let rows = statement.query_map(params![owner_kind, owner_id], |row| {
            Ok((
                (
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ),
                (row.get::<_, String>(0)?, row.get::<_, i64>(4)?),
            ))
        })?;
        for row in rows {
            let (key, value) = row?;
            existing.insert(key, value);
        }
    }
    transaction.execute(
        "DELETE FROM effect_conditions WHERE owner_kind = ?1 AND owner_id = ?2",
        params![owner_kind, owner_id],
    )?;
    let mut identities = Vec::new();
    for proposal in proposals {
        let (subject_kind, subject_id) = subject_parts(&proposal.subject)?;
        let key = (
            subject_kind.to_string(),
            subject_id.clone(),
            proposal.code.as_str().to_string(),
        );
        let (identity, first_observed_at) = existing
            .remove(&key)
            .unwrap_or_else(|| (Uuid::new_v4().to_string(), observed_at.millis()));
        let (retry_kind, retry_version, retry_json) = retry_parts(&proposal.retry)?;
        transaction.execute(
            "INSERT INTO effect_conditions (
                 id, owner_kind, owner_id, subject_kind, subject_id, code, impact,
                 retry_kind, retry_policy_version, retry_policy_json, generation,
                 safe_details_version, safe_details_json, first_observed_at, last_observed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?13, ?14)",
            params![
                &identity,
                owner_kind,
                owner_id,
                subject_kind,
                subject_id,
                proposal.code.as_str(),
                match proposal.impact {
                    ConditionImpact::Blocking => "blocking",
                    ConditionImpact::NonBlocking => "non_blocking",
                },
                retry_kind,
                retry_version,
                retry_json,
                match proposal.generation {
                    ConditionGeneration::Unscoped => None,
                    ConditionGeneration::At(generation) => Some(generation_to_sql(generation)?),
                },
                effect_json(&proposal.safe_details)?,
                first_observed_at,
                observed_at.millis(),
            ],
        )?;
        identities.push(ora_effect::ConditionId::new(identity));
    }
    Ok(identities)
}

/// Loads every current Condition for one Target or Resource owner.
fn load_conditions(
    connection: &Connection,
    owner: &ConditionOwner,
) -> Result<Vec<EffectCondition>, DatabaseError> {
    let (owner_kind, owner_id) = owner_parts(owner);
    let mut statement = connection.prepare(
        "SELECT id, subject_id, code, impact, retry_kind, retry_policy_json,
                generation, safe_details_json, first_observed_at, last_observed_at
         FROM effect_conditions
         WHERE owner_kind = ?1 AND owner_id = ?2 ORDER BY code, subject_kind, subject_id",
    )?;
    let mut rows = statement.query(params![owner_kind, owner_id])?;
    let mut conditions = Vec::new();
    while let Some(row) = rows.next()? {
        let retry_kind = row.get::<_, String>("retry_kind")?;
        let retry = match retry_kind.as_str() {
            "on_change" => ConditionRetry::OnChange,
            "manual" => ConditionRetry::Manual,
            "backoff" => ConditionRetry::Backoff(parse_effect_json(
                row.get::<_, Option<String>>("retry_policy_json")?
                    .ok_or_else(|| {
                        DatabaseError::CorruptEffectState(
                            "backoff Condition lacks policy".to_string(),
                        )
                    })?,
            )?),
            other => {
                return Err(DatabaseError::CorruptEffectState(format!(
                    "unknown Condition retry kind {other}"
                )));
            }
        };
        conditions.push(EffectCondition {
            identity: ora_effect::ConditionId::new(row.get::<_, String>("id")?),
            owner: owner.clone(),
            subject: parse_effect_json(row.get::<_, String>("subject_id")?)?,
            code: StableConditionCode::parse(row.get::<_, String>("code")?)
                .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?,
            impact: match row.get::<_, String>("impact")?.as_str() {
                "blocking" => ConditionImpact::Blocking,
                "non_blocking" => ConditionImpact::NonBlocking,
                other => {
                    return Err(DatabaseError::CorruptEffectState(format!(
                        "unknown Condition impact {other}"
                    )));
                }
            },
            retry,
            generation: row
                .get::<_, Option<i64>>("generation")?
                .map(super::mapping::generation_from_sql)
                .transpose()?
                .map_or(ConditionGeneration::Unscoped, ConditionGeneration::At),
            safe_details: parse_effect_json(row.get::<_, String>("safe_details_json")?)?,
            first_observed_at: LocalTimestamp::from_millis(row.get::<_, i64>("first_observed_at")?),
            last_observed_at: LocalTimestamp::from_millis(row.get::<_, i64>("last_observed_at")?),
        });
    }
    Ok(conditions)
}

/// Maps a typed owner to its polymorphic table discriminator and identity.
fn owner_parts(owner: &ConditionOwner) -> (&'static str, &str) {
    match owner {
        ConditionOwner::Target(target) => ("target", target.as_str()),
        ConditionOwner::Resource(resource) => ("resource", resource.as_str()),
    }
}

/// Stores the full typed subject in JSON while retaining an indexed discriminator.
fn subject_parts(subject: &ConditionSubject) -> Result<(&'static str, String), DatabaseError> {
    let kind = match subject {
        ConditionSubject::Consumer(_) => "consumer",
        ConditionSubject::Target(_) => "target",
        ConditionSubject::DesiredEffect(_) => "desired_effect",
        ConditionSubject::Resource(_) => "resource",
        ConditionSubject::ManagedItem(_) => "managed_item",
        ConditionSubject::Operation(_) => "operation",
        ConditionSubject::Artifact(_) => "artifact",
    };
    Ok((kind, effect_json(subject)?))
}

/// Splits the retry union into constrained SQLite columns.
fn retry_parts(
    retry: &ConditionRetry,
) -> Result<(&'static str, Option<i64>, Option<String>), DatabaseError> {
    match retry {
        ConditionRetry::OnChange => Ok(("on_change", None, None)),
        ConditionRetry::Manual => Ok(("manual", None, None)),
        ConditionRetry::Backoff(policy) => Ok(("backoff", Some(1), Some(effect_json(policy)?))),
    }
}

/// Persists one Target status only through a validated domain snapshot.
pub(super) fn save_target_status(
    transaction: &Transaction<'_>,
    status: &TargetStatus,
) -> Result<(), DatabaseError> {
    let progress = status.progress();
    let (phase, recovery) = target_phase_value(status.phase());
    transaction.execute(
        "UPDATE effect_target_status SET
             desired_generation = ?2, observed_generation = ?3, applied_generation = ?4,
             ready_generation = ?5, phase = ?6, recovery_operation_id = ?7,
             status_version = ?8, updated_at = ?9
         WHERE target_id = ?1",
        params![
            status.target().as_str(),
            generation_to_sql(progress.desired())?,
            generation_to_sql(progress.observed())?,
            generation_to_sql(progress.applied())?,
            generation_to_sql(progress.ready())?,
            phase,
            recovery,
            status_version_to_sql(status.version())?,
            status.updated_at().millis(),
        ],
    )?;
    Ok(())
}

/// Persists one Resource status only through a validated domain snapshot.
pub(super) fn save_resource_status(
    transaction: &Transaction<'_>,
    status: &ResourceStatus,
) -> Result<(), DatabaseError> {
    let (phase, recovery) = resource_phase_value(status.phase());
    transaction.execute(
        "UPDATE effect_resource_status SET
             desired_generation = ?2, observed_generation = ?3, applied_generation = ?4,
             phase = ?5, recovery_operation_id = ?6, status_version = ?7, updated_at = ?8
         WHERE resource_id = ?1",
        params![
            status.resource().as_str(),
            generation_to_sql(status.desired())?,
            generation_to_sql(status.observed())?,
            generation_to_sql(status.applied())?,
            phase,
            recovery,
            status_version_to_sql(status.version())?,
            status.updated_at().millis(),
        ],
    )?;
    Ok(())
}
