use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions ADD COLUMN mcp_selection TEXT NOT NULL
    DEFAULT '{"mode":"explicit","pluginIds":[]}' CHECK (json_valid(mcp_selection));
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions DROP COLUMN mcp_selection;
"#];

/// Persists session MCP authority without granting permissions to existing sessions.
pub fn migration() -> Migration {
    Migration::new("0011", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use super::{DOWN_STATEMENTS, UP_STATEMENTS};
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;

    /// Legacy sessions and inserts omitting selection deny MCP access without workflow metadata.
    #[test]
    fn defaults_session_mcp_selection_to_empty_explicit_and_rolls_back() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE sessions (id TEXT PRIMARY KEY);
                INSERT INTO sessions (id) VALUES ('ordinary'), ('workflow');
                "#,
            )
            .unwrap();

        connection.execute_batch(UP_STATEMENTS[0]).unwrap();
        connection
            .execute("INSERT INTO sessions (id) VALUES ('defaulted')", [])
            .unwrap();

        let rows = connection
            .prepare("SELECT id, mcp_selection FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "defaulted".to_string(),
                    r#"{"mode":"explicit","pluginIds":[]}"#.to_string()
                ),
                (
                    "ordinary".to_string(),
                    r#"{"mode":"explicit","pluginIds":[]}"#.to_string()
                ),
                (
                    "workflow".to_string(),
                    r#"{"mode":"explicit","pluginIds":[]}"#.to_string()
                ),
            ]
        );

        connection.execute_batch(DOWN_STATEMENTS[0]).unwrap();
        let columns = connection
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(columns, vec!["id"]);
    }
}
