use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};

/// Names the supported SQLite storage modes without relying on boolean configuration flags.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DatabaseLocation {
    Path(PathBuf),
    InMemory,
}

impl DatabaseLocation {
    /// Builds a file-backed location from a caller-provided path.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }

    /// Builds an isolated in-memory database location suitable for tests.
    pub fn in_memory() -> Self {
        Self::InMemory
    }

    /// Opens a SQLite connection with flags that match the selected storage mode.
    pub(crate) fn open(&self) -> Result<Connection, rusqlite::Error> {
        let connection = match self {
            Self::Path(path) => Connection::open(path),
            Self::InMemory => Connection::open_with_flags(
                ":memory:",
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            ),
        }?;
        register_debug_hash(&connection)?;
        Ok(connection)
    }
}

/// Installs the deterministic digest used by debug-evidence triggers on a writer.
pub(crate) fn register_debug_hash(connection: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    use sha2::{Digest, Sha256};
    connection.create_scalar_function(
        "ora_debug_hash",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let text: Option<String> = context.get(0)?;
            Ok(text.map(|value| Sha256::digest(value.as_bytes()).to_vec()))
        },
    )
}
