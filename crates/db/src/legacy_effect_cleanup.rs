use crate::DatabaseError;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

/// Persisted legacy `ora/skill-directory.v1` record read before migration `0025` drops Effect tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyManagedSkillRow {
    pub managed_identity: String,
    pub resource_id: String,
    pub native_identity: String,
    pub fingerprint: String,
    pub descriptor_json: String,
}

/// Reads legacy Effect-managed skill directory rows when Effect tables still exist on disk.
///
/// Missing databases and post-`0025` databases return an empty list without creating files or
/// mutating state so startup can run cleanup strictly before schema migrations execute.
pub fn read_legacy_managed_skills(
    database_path: &Path,
) -> Result<Vec<LegacyManagedSkillRow>, DatabaseError> {
    if !database_path.exists() {
        return Ok(Vec::new());
    }
    let connection = Connection::open(database_path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    if !has_table(&connection, "effect_managed_items")?
        || !has_table(&connection, "effect_resources")?
    {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT item.id, item.resource_id, item.native_identity, item.fingerprint, resource.descriptor_json
         FROM effect_managed_items item
         JOIN effect_resources resource ON resource.id = item.resource_id
         WHERE resource.adapter_id = 'ora/filesystem-directory'
           AND resource.materialization_format = 'ora/skill-directory.v1'
         ORDER BY item.resource_id, item.native_identity, item.id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(LegacyManagedSkillRow {
                managed_identity: row.get(0)?,
                resource_id: row.get(1)?,
                native_identity: row.get(2)?,
                fingerprint: row.get(3)?,
                descriptor_json: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn has_table(connection: &Connection, table_name: &str) -> Result<bool, DatabaseError> {
    let found: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table_name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}
