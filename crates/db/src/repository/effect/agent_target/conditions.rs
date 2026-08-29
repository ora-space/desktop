//! Agent Target-owned condition load and replace.

use super::encode::*;
use crate::DatabaseError;
use ora_effect::{
    AgentTargetCondition, AgentTargetConditionAttachment, AgentTargetConditionSubject, ConsumerId,
    SurfaceKey,
};
use rusqlite::params;
use uuid::Uuid;

/// Loads every condition owned by one target in a stable order for whole-object comparison.
pub(super) fn load_target_conditions(
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
        let attachment = match (surface_id, consumer_id) {
            (Some(surface_key), consumer_id) => Some(AgentTargetConditionAttachment {
                surface_key: SurfaceKey::new(surface_key),
                consumer_id: consumer_id.map(ConsumerId::new),
            }),
            (None, None) => None,
            (None, Some(_)) => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(DatabaseError::CorruptEffectState(
                        "condition consumer attachment is missing its surface".to_string(),
                    )),
                ));
            }
        };
        Ok(AgentTargetCondition {
            id: row.get(0)?,
            subject,
            reason,
            impact,
            message: row.get(8)?,
            first_observed_at: row.get(9)?,
            last_observed_at: row.get(10)?,
            failed_generation,
            attachment,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Replaces the complete condition set so a status write cannot leave a mixed generation.
pub(super) fn replace_target_conditions(
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
        let (surface_id, consumer_id) = match &condition.attachment {
            Some(attachment) => (
                Some(attachment.surface_key.as_str()),
                attachment.consumer_id.as_ref().map(ConsumerId::as_str),
            ),
            None => (None, None),
        };
        transaction.execute(
            "INSERT INTO effect_agent_target_conditions (
                 id, agent_target_id, surface_id, consumer_id, subject_kind, subject_id, reason,
                 impact, failed_generation, message, first_observed_at, last_observed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                agent_target_id,
                surface_id,
                consumer_id,
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
