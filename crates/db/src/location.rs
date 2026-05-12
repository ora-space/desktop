use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::DatabaseError;

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
    pub fn open(&self) -> Result<Connection, DatabaseError> {
        match self {
            Self::Path(path) => Ok(Connection::open(path)?),
            Self::InMemory => Ok(Connection::open_with_flags(
                ":memory:",
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            )?),
        }
    }

    /// Returns the stable location label used by operational logs.
    pub(crate) fn logging_label(&self) -> &str {
        match self {
            Self::Path(_) => "path",
            Self::InMemory => "in_memory",
        }
    }

    /// Returns the backing path required by file-based pooling entry points.
    pub(crate) fn pooled_path(&self) -> Result<&Path, DatabaseError> {
        match self {
            Self::Path(path) => Ok(path.as_path()),
            Self::InMemory => Err(DatabaseError::UnsupportedPooledLocation),
        }
    }
}
