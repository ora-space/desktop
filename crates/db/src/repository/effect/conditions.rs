//! Current Condition identity and observation history, independent of status audit timestamps.
use super::mapping::{effect_json, generation_to_sql, parse_effect_json};
use crate::DatabaseError;
use ora_effect::{
    ConditionGeneration, ConditionImpact, ConditionOwner, ConditionProposal, ConditionRetry,
    ConditionSubject, EffectCondition, LocalTimestamp, StableConditionCode,
};
use rusqlite::{Connection, Transaction, params};
use std::collections::BTreeMap;
use uuid::Uuid;

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
            "SELECT id, subject_kind, subject_id, code, first_observed_at, last_observed_at
             FROM effect_conditions WHERE owner_kind = ?1 AND owner_id = ?2",
        )?;
        let rows = statement.query_map(params![owner_kind, owner_id], |row| {
            Ok((
                (
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ),
                (
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ),
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
        let (identity, first_observed_at, last_observed_at) =
            existing.remove(&key).unwrap_or_else(|| {
                (
                    Uuid::new_v4().to_string(),
                    observed_at.millis(),
                    observed_at.millis(),
                )
            });
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
                last_observed_at.max(observed_at.millis()),
            ],
        )?;
        identities.push(ora_effect::ConditionId::new(identity));
    }
    Ok(identities)
}

/// Loads every current Condition for one Target or Resource owner.
pub(super) fn load_conditions(
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
