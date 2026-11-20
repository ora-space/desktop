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
    host: &crate::SessionStopHost,
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
    if app != APPLICATION_ID || !matches!(version, 1..=16) {
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
    if version >= 10 {
        expected.execute_batch(include_str!("branch_heads.sql"))?;
    }
    if version >= 11 {
        expected.execute_batch(include_str!("restore.sql"))?;
    }
    if version >= 12 {
        expected.execute_batch(include_str!("scheduled_artifacts.sql"))?;
    }
    if version >= 13 {
        expected.execute_batch(include_str!("direct_git.sql"))?;
    }
    if version >= 14 {
        expected.execute_batch(include_str!("session_stop.sql"))?;
    }
    if version >= 15 {
        expected.execute_batch(include_str!("turn_cancel.sql"))?;
    }
    if version >= 16 {
        expected.execute_batch(include_str!("session_stop_marks.sql"))?;
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
    // Inspect the old schema before the first migration, including upgrades through older versions.
    let legacy_sessions = crate::session_stop::legacy_running(connection, version)?;
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
    if version < 10 {
        // v10 adds the own-branch ledger. Clone, session and delivery records written before
        // the switch to Git Server snapshots are not migrated: their inputs no longer decode, and
        // the Workspaces they belong to are rebuilt rather than resumed.
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("branch_heads.sql"))?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 10,
        )?;
        tx.commit()?;
    }
    if version < 11 {
        // v11 records restore admission, original restore identities and deferred stop evidence.
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("restore.sql"))?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 11,
        )?;
        tx.commit()?;
    }
    if version < 12 {
        // v12 adds scheduled-artifact uploads and widens the terminal-event outbox to every family.
        // Both rebuild tables referenced by existing records, so enforcement is disabled outside the
        // transaction and its result is validated before commit.
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "OFF")?;
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("scheduled_artifacts.sql"))?;
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
            /*pragma_value*/ 12,
        )?;
        tx.commit()?;
    }
    if version < 13 {
        // v13 adds the ending "stopped without an exit status" and gives it to every attempt the
        // retired process host left unsettled. Deployment ends the old Node, host and guardian
        // tree before this program starts, so none of those commands can still be writing; their
        // exit status is gone with the host, and a restore's stop counts as confirmed. No
        // existing column is rewritten.
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("direct_git.sql"))?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 13,
        )?;
        tx.commit()?;
    }
    if version < 14 {
        crate::session_stop::migrate(connection, legacy_sessions, &id, host)?;
    }
    if version < 15 {
        // v15 records the one settlement each accepted `CancelTurn` receives. It only adds a
        // table; older programs reject a v15 file instead of replaying commands they cannot read.
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("turn_cancel.sql"))?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 15,
        )?;
        tx.commit()?;
    }
    if version < 16 {
        // v16 admits the stop proof `lock_processes_and_marks`, recorded once the attempt mark
        // scan also came back clean. SQLite cannot widen a CHECK in place, so the table is
        // rebuilt with every row and value unchanged; older programs reject a v16 file instead
        // of misreading the new proof.
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("session_stop_marks.sql"))?;
        tx.pragma_update(
            /*schema_name*/ None,
            "user_version",
            /*pragma_value*/ 16,
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
