use std::fmt;

use thiserror::Error;

/// Reports whether a migration step was moving schema state forward or backward.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationDirection {
    Up,
    Down,
}

impl fmt::Display for MigrationDirection {
    /// Formats the direction in the same vocabulary used by migration authors and logs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Up => formatter.write_str("up"),
            Self::Down => formatter.write_str("down"),
        }
    }
}

/// Describes failures that can occur while validating or reconciling database migrations.
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("connection pool error: {0}")]
    ConnectionPool(#[from] r2d2::Error),
    #[error("domain model error: {0}")]
    DomainModel(#[from] ora_domain::DomainModelError),
    #[error("migration versions must be unique, found duplicate version `{0}`")]
    DuplicateMigrationVersion(String),
    #[error("migration versions must be strictly increasing, found `{current}` after `{previous}`")]
    UnorderedMigrationVersions { previous: String, current: String },
    #[error(
        "target migration versions must match the catalog prefix at position {position}: expected `{expected}`, found `{found}`"
    )]
    InvalidTargetVersionPrefix {
        position: usize,
        expected: String,
        found: String,
    },
    #[error("database contains applied migration `{version}` that is not defined in the catalog")]
    UnknownAppliedMigrationVersion { version: String },
    #[error(
        "database migration history diverged at position {position}: expected `{expected}`, found `{found}`"
    )]
    DivergedMigrationHistory {
        position: usize,
        expected: String,
        found: String,
    },
    #[error("failed to execute migration `{version}` {direction}")]
    MigrationStepFailed {
        version: String,
        direction: MigrationDirection,
        #[source]
        source: rusqlite::Error,
    },
    #[error("pooled sqlite connections require a file-backed database location")]
    UnsupportedPooledLocation,
}
