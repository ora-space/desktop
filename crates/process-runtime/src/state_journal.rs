use crate::ProcessStateError;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// Rejects engines without the selected mainline WAL-reset fix before touching state files.
pub(crate) fn check_engine() -> Result<(), ProcessStateError> {
    if rusqlite::version_number() < 3_051_003 {
        return Err(ProcessStateError::Rejected(
            "SQLite engine predates required WAL fixes",
        ));
    }
    Ok(())
}

/// Opens only an existing journal file and verifies the process subsystem's WAL/FULL policy.
pub(crate) fn open_writable(path: &Path) -> Result<Connection, ProcessStateError> {
    check_engine()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(Duration::ZERO)?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")?;
    let settings = connection.query_row("SELECT (SELECT journal_mode FROM pragma_journal_mode), (SELECT synchronous FROM pragma_synchronous)",
        [], |row| Ok((row.get::<_, String>(/*idx*/ 0)?, row.get::<_, i64>(/*idx*/ 1)?)))?;
    if settings != ("wal".to_owned(), 2) {
        return Err(ProcessStateError::Rejected(
            "journal did not enable WAL and FULL",
        ));
    }
    Ok(connection)
}
