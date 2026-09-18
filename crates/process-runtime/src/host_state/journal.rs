use std::num::NonZeroU64;
use std::path::Path;

use ora_process_protocol::{HostBinding, ScopeCreationIntent, ScopeId};
use rusqlite::{Connection, OpenFlags, params};

use super::ProcessStateError;

const APPLICATION_ID: i64 = 0x4f52_4148;
const VERSION: i64 = 6;
const LEGACY_LAUNCH_SCHEMA: &str = "CREATE TABLE guardian_launches (
    scope TEXT PRIMARY KEY NOT NULL REFERENCES scope_intents(scope),
    credential BLOB NOT NULL CHECK (length(credential) = 32),
    phase TEXT NOT NULL CHECK (phase = 'launch_unknown')
) STRICT";
const LAUNCH_SCHEMA: &str = "CREATE TABLE guardian_launches (
    scope TEXT PRIMARY KEY NOT NULL REFERENCES scope_intents(scope),
    phase TEXT NOT NULL CHECK (phase = 'launch_unknown')
) STRICT";
const HOST_SCHEMA: &str = "CREATE TABLE host_binding (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    epoch INTEGER NOT NULL CHECK (epoch > 0),
    instance TEXT NOT NULL
) STRICT";
const INTENT_SCHEMA: &str = "CREATE TABLE scope_intents (
    scope TEXT PRIMARY KEY NOT NULL,
    guardian TEXT NOT NULL UNIQUE,
    host_epoch INTEGER NOT NULL CHECK (host_epoch > 0),
    host_instance TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase = 'intent_recorded')
) STRICT";

/// Commits the format identity, initial host incarnation and empty intent journal atomically.
pub(super) fn initialize(
    connection: &mut Connection,
    binding: HostBinding,
) -> Result<(), ProcessStateError> {
    let transaction = connection.transaction()?;
    transaction.pragma_update(/*schema_name*/ None, "application_id", APPLICATION_ID)?;
    transaction.pragma_update(/*schema_name*/ None, "user_version", VERSION)?;
    transaction.execute_batch(HOST_SCHEMA)?;
    transaction.execute_batch(INTENT_SCHEMA)?;
    transaction.execute_batch(LAUNCH_SCHEMA)?;
    transaction.execute_batch(super::runs::SCHEMA)?;
    transaction.execute_batch(super::control::STOP_SCHEMA)?;
    transaction.execute_batch(super::control::CLOSE_SCHEMA)?;
    transaction.execute_batch(super::observations::RUN_SCHEMA)?;
    transaction.execute_batch(super::observations::SCOPE_SCHEMA)?;
    transaction.execute(
        "INSERT INTO host_binding VALUES (1, ?1, ?2)",
        params![epoch_to_sql(binding.epoch)?, binding.instance.to_string()],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Checks the complete supported schema and identities without repairing or initializing anything.
pub(super) fn inspect(
    path: &Path,
    existing_scopes: &[ScopeId],
) -> Result<(HostBinding, i64), ProcessStateError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.execute_batch("PRAGMA trusted_schema=OFF;")?;
    let header = connection.query_row(
        "SELECT (SELECT application_id FROM pragma_application_id), (SELECT user_version FROM pragma_user_version)",
        [], |row| Ok((row.get::<_, i64>(/*idx*/ 0)?, row.get::<_, i64>(/*idx*/ 1)?)),
    )?;
    if header.0 != APPLICATION_ID || !matches!(header.1, 1 | 2 | 3 | 4 | 5 | VERSION) {
        return Err(ProcessStateError::Rejected(
            "unknown journal identity or version",
        ));
    }
    let definitions = connection
        .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name")?
        .query_map([], |row| row.get::<_, String>(/*idx*/ 0))?
        .collect::<Result<Vec<_>, _>>()?;
    let expected = if header.1 == 1 {
        vec![HOST_SCHEMA, INTENT_SCHEMA]
    } else if header.1 == 2 {
        vec![LEGACY_LAUNCH_SCHEMA, HOST_SCHEMA, INTENT_SCHEMA]
    } else if header.1 == 3 {
        vec![LAUNCH_SCHEMA, HOST_SCHEMA, INTENT_SCHEMA]
    } else if header.1 == 4 {
        vec![
            LAUNCH_SCHEMA,
            HOST_SCHEMA,
            super::runs::SCHEMA,
            INTENT_SCHEMA,
        ]
    } else if header.1 == 5 {
        vec![
            LAUNCH_SCHEMA,
            HOST_SCHEMA,
            super::runs::SCHEMA,
            super::control::STOP_SCHEMA,
            super::control::CLOSE_SCHEMA,
            INTENT_SCHEMA,
        ]
    } else {
        vec![
            LAUNCH_SCHEMA,
            HOST_SCHEMA,
            super::runs::SCHEMA,
            super::observations::RUN_SCHEMA,
            super::control::STOP_SCHEMA,
            super::control::CLOSE_SCHEMA,
            INTENT_SCHEMA,
            super::observations::SCOPE_SCHEMA,
        ]
    };
    if definitions != expected {
        return Err(ProcessStateError::Rejected(
            "journal schema does not match its version",
        ));
    }
    let integrity: String =
        connection.query_row("PRAGMA quick_check", [], |row| row.get(/*idx*/ 0))?;
    if integrity != "ok" {
        return Err(ProcessStateError::Rejected(
            "journal integrity check failed",
        ));
    }
    let (epoch, instance) = connection.query_row(
        "SELECT epoch, instance FROM host_binding WHERE singleton=1",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(/*idx*/ 0)?,
                row.get::<_, String>(/*idx*/ 1)?,
            ))
        },
    )?;
    let binding = HostBinding {
        epoch: epoch_from_sql(epoch)?,
        instance: instance.parse()?,
    };
    let mut statement = connection
        .prepare("SELECT scope, guardian, host_epoch, host_instance, phase FROM scope_intents")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let intent = decode_intent(row)?;
        if intent.created_by.epoch > binding.epoch
            || (intent.created_by.epoch == binding.epoch
                && intent.created_by.instance != binding.instance)
        {
            return Err(ProcessStateError::Rejected(
                "creation intent contradicts host binding",
            ));
        }
    }
    for scope in existing_scopes {
        if find_intent(&connection, *scope)?.is_none() {
            return Err(ProcessStateError::Rejected(
                "scope directory has no recorded creation responsibility",
            ));
        }
    }
    if header.1 >= 2 {
        let orphaned: i64 = connection.query_row("SELECT count(*) FROM guardian_launches WHERE scope NOT IN (SELECT scope FROM scope_intents)", [], |row| row.get(/*idx*/ 0))?;
        if orphaned != 0 {
            return Err(ProcessStateError::Rejected(
                "orphaned guardian launch record",
            ));
        }
    }
    if header.1 >= 4 {
        super::runs::inspect(&connection)?;
    }
    if header.1 >= 5 {
        super::control::inspect(&connection)?;
    }
    if header.1 == VERSION {
        super::observations::inspect(&connection)?;
    }
    Ok((binding, header.1))
}

/// Advances only host ownership; original scope creation identities are never rewritten on recovery.
pub(super) fn advance_binding(
    connection: &mut Connection,
    binding: HostBinding,
    version: i64,
) -> Result<(), ProcessStateError> {
    let transaction = connection.transaction()?;
    // v1 could not launch guardians. Add the new responsibility table without rewriting intents;
    // schema version and new host binding commit together, so old binaries fail closed afterward.
    if version == 1 {
        transaction.execute_batch(LAUNCH_SCHEMA)?;
    } else if version == 2 {
        // Rebuild only the known table, preserving consumed attempts and exact current schema.
        transaction
            .execute_batch("ALTER TABLE guardian_launches RENAME TO legacy_guardian_launches;")?;
        transaction.execute_batch(LAUNCH_SCHEMA)?;
        transaction.execute_batch("INSERT INTO guardian_launches SELECT scope, phase FROM legacy_guardian_launches; DROP TABLE legacy_guardian_launches;")?;
    }
    if version < 4 {
        // Old guardians retain their own facts; do not invent host intents for historical Runs.
        transaction.execute_batch(super::runs::SCHEMA)?;
    }
    if version < 5 {
        transaction.execute_batch(super::control::STOP_SCHEMA)?;
        transaction.execute_batch(super::control::CLOSE_SCHEMA)?;
    }
    if version < VERSION {
        transaction.execute_batch(super::observations::RUN_SCHEMA)?;
        transaction.execute_batch(super::observations::SCOPE_SCHEMA)?;
        transaction.pragma_update(/*schema_name*/ None, "user_version", VERSION)?;
    }
    let changed = transaction.execute(
        "UPDATE host_binding SET epoch=?1, instance=?2 WHERE singleton=1",
        params![epoch_to_sql(binding.epoch)?, binding.instance.to_string()],
    )?;
    if changed != 1 {
        return Err(ProcessStateError::Rejected("missing host binding"));
    }
    transaction.commit()?;
    Ok(())
}

/// Loads one exact record; absence is a query result, never permission to recreate a guardian.
pub(super) fn find_intent(
    connection: &Connection,
    scope: ScopeId,
) -> Result<Option<ScopeCreationIntent>, ProcessStateError> {
    let mut statement = connection.prepare("SELECT scope, guardian, host_epoch, host_instance, phase FROM scope_intents WHERE scope=?1")?;
    let mut rows = statement.query([scope.to_string()])?;
    rows.next()?.map(decode_intent).transpose()
}

/// Commits the original responsibility before returning it to the caller.
pub(super) fn insert_intent(
    connection: &mut Connection,
    intent: &ScopeCreationIntent,
) -> Result<(), ProcessStateError> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO scope_intents VALUES (?1, ?2, ?3, ?4, 'intent_recorded')",
        params![
            intent.scope.to_string(),
            intent.guardian.to_string(),
            epoch_to_sql(intent.created_by.epoch)?,
            intent.created_by.instance.to_string()
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Refuses unknown phases and malformed identities rather than treating them as absent records.
fn decode_intent(row: &rusqlite::Row<'_>) -> Result<ScopeCreationIntent, ProcessStateError> {
    let phase: String = row.get(/*idx*/ 4)?;
    if phase != "intent_recorded" {
        return Err(ProcessStateError::Rejected(
            "unknown guardian creation phase",
        ));
    }
    Ok(ScopeCreationIntent {
        scope: row.get::<_, String>(/*idx*/ 0)?.parse()?,
        guardian: row.get::<_, String>(/*idx*/ 1)?.parse()?,
        created_by: HostBinding {
            epoch: epoch_from_sql(row.get(/*idx*/ 2)?)?,
            instance: row.get::<_, String>(/*idx*/ 3)?.parse()?,
        },
    })
}

/// Prevents an unsigned protocol epoch from wrapping SQLite's signed integer range.
fn epoch_to_sql(epoch: NonZeroU64) -> Result<i64, ProcessStateError> {
    i64::try_from(epoch.get())
        .ok()
        .filter(|epoch| *epoch > 0)
        .ok_or(ProcessStateError::Rejected(
            "host epoch outside journal range",
        ))
}

/// Rejects corrupt or zero stored epochs instead of coercing them into a new incarnation.
fn epoch_from_sql(epoch: i64) -> Result<NonZeroU64, ProcessStateError> {
    u64::try_from(epoch)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(ProcessStateError::Rejected("invalid stored host epoch"))
}
