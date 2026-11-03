use super::conditions::load_conditions;
use super::mapping::load_target_status;
use crate::DatabaseError;
use ora_effect::{
    ConditionOwner, EffectResourceId, EffectTargetId, LocalTimestamp, TargetStatusView,
};
use rusqlite::{Transaction, params};

/// Keeps audit metadata outside domain transitions and reads all evidence in one snapshot.
pub(super) fn load_target_view(
    transaction: &Transaction<'_>,
    target: &EffectTargetId,
) -> Result<Option<TargetStatusView>, DatabaseError> {
    let Some(status) = load_target_status(transaction, target)? else {
        return Ok(None);
    };
    let updated_at = transaction.query_row(
        "SELECT updated_at FROM effect_target_status WHERE target_id = ?1",
        params![target.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let mut conditions = load_conditions(transaction, &ConditionOwner::Target(target.clone()))?;
    let resource_ids = {
        let mut statement = transaction.prepare(
            "SELECT resource_id
             FROM effect_target_resource_bindings
             WHERE target_id = ?1
             ORDER BY resource_id",
        )?;
        let rows = statement.query_map(params![target.as_str()], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for resource_id in resource_ids {
        conditions.extend(load_conditions(
            transaction,
            &ConditionOwner::Resource(EffectResourceId::new(resource_id)),
        )?);
    }
    Ok(Some(TargetStatusView {
        status,
        updated_at: LocalTimestamp::from_millis(updated_at),
        conditions,
    }))
}
