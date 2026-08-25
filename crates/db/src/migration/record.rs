/// Represents one applied migration row loaded from the SQLite bookkeeping table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedMigration {
    pub version: String,
    pub up_sql: String,
    pub down_sql: String,
    pub executed_at: i64,
}

impl AppliedMigration {
    /// Builds a testable value object from the persisted version, SQL snapshots, and timestamp.
    pub fn new(
        version: impl Into<String>,
        up_sql: impl Into<String>,
        down_sql: impl Into<String>,
        executed_at: i64,
    ) -> Self {
        Self {
            version: version.into(),
            up_sql: up_sql.into(),
            down_sql: down_sql.into(),
            executed_at,
        }
    }
}
