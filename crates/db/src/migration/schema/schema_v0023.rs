use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions ADD COLUMN host_capabilities TEXT NOT NULL DEFAULT 'none'
    CHECK (host_capabilities IN ('none', 'workflow_completion'));
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions DROP COLUMN host_capabilities;
"#];

/// Persists the host-only capabilities a session was created with.
///
/// A workflow-owned session may carry the completion MCP capability; ordinary sessions do not.
/// The flag has to survive an application restart because the runtime lazily restores actors from
/// storage: deriving it from the live actor would silently drop the capability after a restart.
pub fn migration() -> Migration {
    Migration::new("0023", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::UP_STATEMENTS;

    #[test]
    fn defaults_existing_sessions_to_no_host_capabilities() {
        let connection = Connection::open_in_memory().expect("open database");
        connection
            .execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY);")
            .expect("create sessions");
        for statement in UP_STATEMENTS {
            connection
                .execute_batch(statement)
                .expect("apply migration");
        }
        connection
            .execute("INSERT INTO sessions (id) VALUES ('session-1')", [])
            .expect("insert legacy session");
        let value: String = connection
            .query_row(
                "SELECT host_capabilities FROM sessions WHERE id = 'session-1'",
                [],
                |row| row.get(0),
            )
            .expect("read default");
        assert_eq!(value, "none");
    }
}
