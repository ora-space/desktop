use pretty_assertions::assert_eq;

use crate::{DatabaseLocation, RepositoryPool};

/// Checks the linked engine, not Cargo metadata, so overrides cannot silently restore the WAL bug.
#[test]
fn linked_sqlite_includes_wal_reset_fix() -> Result<(), crate::DatabaseError> {
    let pool = RepositoryPool::new(&DatabaseLocation::in_memory())?;
    let (version, source_id) = pool.with_connection(|connection| {
        Ok(
            connection.query_row("SELECT sqlite_version(), sqlite_source_id()", [], |row| {
                Ok((
                    row.get::<_, String>(/*idx*/ 0)?,
                    row.get::<_, String>(/*idx*/ 1)?,
                ))
            })?,
        )
    })?;

    assert_eq!(version, rusqlite::version());
    // Ora selects the main release line, not older branches with separately backported fixes.
    // https://sqlite.org/wal.html documents the WAL-reset fix in 3.51.3 and later.
    assert!(
        rusqlite::version_number() >= 3_051_003,
        "SQLite {version} ({source_id}) predates the required WAL-reset fix"
    );
    println!("linked SQLite {version}; source_id={source_id}");
    Ok(())
}

/// Exercises file-backed pooled connections across commit, rollback, checkpoint and reopen.
#[test]
fn upgraded_engine_preserves_committed_wal_data() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let location = DatabaseLocation::path(directory.path().join("engine.sqlite"));
    let pool = RepositoryPool::new(&location)?;
    pool.with_connection_mut(|writer| {
        let settings = writer.query_row(
            "SELECT (SELECT journal_mode FROM pragma_journal_mode),
                    (SELECT synchronous FROM pragma_synchronous),
                    (SELECT foreign_keys FROM pragma_foreign_keys)",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(/*idx*/ 0)?,
                    row.get::<_, i64>(/*idx*/ 1)?,
                    row.get::<_, i64>(/*idx*/ 2)?,
                ))
            },
        )?;
        assert_eq!(settings, ("wal".to_owned(), 1, 1));
        writer
            .execute_batch("CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")?;
        let transaction = writer.transaction()?;
        transaction.execute("INSERT INTO records VALUES (1, 'committed')", [])?;
        transaction.commit()?;

        let transaction = writer.transaction()?;
        transaction.execute("UPDATE records SET value = 'uncommitted'", [])?;
        // Holding the writer forces a distinct pooled connection to observe only committed data.
        pool.with_connection(|reader| {
            let value: String = reader.query_row("SELECT value FROM records", [], |row| {
                row.get(/*idx*/ 0)
            })?;
            assert_eq!(value, "committed");
            Ok(())
        })?;
        transaction.rollback()?;
        let checkpoint = writer.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(/*idx*/ 0)?,
                row.get::<_, i64>(/*idx*/ 1)?,
                row.get::<_, i64>(/*idx*/ 2)?,
            ))
        })?;
        assert_eq!(checkpoint, (0, 0, 0));
        Ok(())
    })?;
    drop(pool);

    let reopened = RepositoryPool::new(&location)?;
    reopened.with_connection(|connection| {
        let record = connection.query_row("SELECT id, value FROM records", [], |row| {
            Ok((
                row.get::<_, i64>(/*idx*/ 0)?,
                row.get::<_, String>(/*idx*/ 1)?,
            ))
        })?;
        assert_eq!(record, (1, "committed".to_owned()));
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(/*idx*/ 0))?;
        assert_eq!(integrity, "ok");
        Ok(())
    })?;
    Ok(())
}
