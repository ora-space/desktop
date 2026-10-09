use crate::{Error, NodeIdentity};
use ora_node_protocol::NodeId;
use rusqlite::Connection;

const APPLICATION_ID: i64 = 0x4f52414e;
const METADATA: &str = "CREATE TABLE node_metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), node_id TEXT NOT NULL CHECK(length(node_id) > 0));";

/// Checks identity before any persistent pragma or migration touches an existing database.
pub(super) fn initialize(
    connection: &mut Connection,
    created: bool,
    identity: &NodeIdentity,
) -> Result<NodeId, Error> {
    if created {
        let id = match identity {
            NodeIdentity::Discover => NodeId::new(uuid::Uuid::new_v4().to_string()),
            NodeIdentity::Require(id) => id.clone(),
        };
        if id.as_str().trim().is_empty() {
            return Err(Error::NodeMismatch);
        }
        let tx = connection.transaction()?;
        tx.pragma_update(/*schema_name*/ None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 1,
        )?;
        tx.execute_batch(METADATA)?;
        tx.execute("INSERT INTO node_metadata VALUES (1, ?1)", [id.as_str()])?;
        tx.execute_batch(include_str!("schema.sql"))?;
        tx.commit()?;
    }
    let app: i64 = connection.pragma_query_value(
        /*schema_name*/ None,
        "application_id",
        |row| row.get(/*idx*/ 0),
    )?;
    let version: i64 = connection.pragma_query_value(
        /*schema_name*/ None,
        "user_version",
        |row| row.get(/*idx*/ 0),
    )?;
    if app != APPLICATION_ID || !matches!(version, 1..=9) {
        return Err(Error::InvalidSchema);
    }
    let check: String = connection.pragma_query_value(
        /*schema_name*/ None,
        "integrity_check",
        |row| row.get(/*idx*/ 0),
    )?;
    if check != "ok" {
        return Err(Error::InvalidSchema);
    }
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(METADATA)?;
    expected.execute_batch(include_str!("schema.sql"))?;
    if version >= 2 {
        expected.execute_batch(include_str!("process.sql"))?;
    }
    if version >= 3 {
        expected.execute_batch(include_str!("repository.sql"))?;
    }
    if version >= 4 {
        expected.execute_batch(include_str!("controller.sql"))?;
    }
    if version >= 5 {
        expected.execute_batch(include_str!("termination.sql"))?;
    }
    if version >= 6 {
        expected.execute_batch(include_str!("runtime_control.sql"))?;
    }
    if version >= 7 {
        expected.execute_batch(include_str!("plugin.sql"))?;
    }
    if version >= 8 {
        expected.execute_batch(include_str!("session.sql"))?;
    }
    if version >= 9 {
        expected.execute_batch(include_str!("revision.sql"))?;
    }
    if schema_objects(connection)? != schema_objects(&expected)? {
        return Err(Error::InvalidSchema);
    }
    let foreign_key_errors: i64 =
        connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(/*idx*/ 0)
        })?;
    if foreign_key_errors != 0 {
        return Err(Error::InvalidSchema);
    }
    let id = NodeId::new(connection.query_row(
        "SELECT node_id FROM node_metadata WHERE singleton = 1",
        [],
        |row| row.get::<_, String>(/*idx*/ 0),
    )?);
    if id.as_str().trim().is_empty()
        || matches!(identity, NodeIdentity::Require(expected) if expected != &id)
    {
        return Err(Error::NodeMismatch);
    }
    if version < 5 {
        let tx = connection.transaction()?;
        if version == 1 {
            tx.execute_batch(include_str!("process.sql"))?;
        }
        if version < 3 {
            tx.execute_batch(include_str!("repository.sql"))?;
        }
        if version < 4 {
            tx.execute_batch(include_str!("controller.sql"))?;
        }
        // v5 only adds signal-termination evidence; no existing row is rewritten, and older
        // programs reject a v5 file instead of misreading a run that has no exit code.
        tx.execute_batch(include_str!("termination.sql"))?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 5,
        )?;
        tx.commit()?;
    }
    if version < 6 {
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("runtime_control.sql"))?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
    }
    if version < 7 {
        // SQLite builds may enable foreign keys by default. Rebuilding the referenced identity
        // table requires disabling enforcement outside the transaction; validate it before commit.
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "OFF")?;
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("plugin.sql"))?;
        let violations: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if violations != 0 {
            return Err(Error::InvalidSchema);
        }
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 7,
        )?;
        tx.commit()?;
    }
    if version < 8 {
        // SQLite builds may enable foreign keys by default. Rebuilding the referenced identity
        // table requires disabling enforcement outside the transaction; validate it before commit.
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "OFF")?;
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("session.sql"))?;
        let violations: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if violations != 0 {
            return Err(Error::InvalidSchema);
        }
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 8,
        )?;
        tx.commit()?;
    }
    if version < 9 {
        // v9 adds Revision delivery as its own execution family; like v7 and v8 it rebuilds the
        // referenced identity table, so enforcement is disabled outside the transaction.
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "OFF")?;
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("revision.sql"))?;
        let violations: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if violations != 0 {
            return Err(Error::InvalidSchema);
        }
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 9,
        )?;
        tx.commit()?;
    }
    Ok(id)
}

/// Compares table and index definitions so an application ID alone cannot authorize an unknown schema.
fn schema_objects(connection: &Connection) -> Result<Vec<(String, String, String)>, Error> {
    let mut statement = connection.prepare(
        "SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
    )?;
    Ok(statement
        .query_map([], |row| {
            Ok((
                row.get(/*idx*/ 0)?,
                row.get(/*idx*/ 1)?,
                row.get(/*idx*/ 2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?)
}
