use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE plugin_marketplace_source
    ADD COLUMN artifact_retrieval TEXT NOT NULL
        DEFAULT '{"type":"direct_https"}'
        CHECK (
            CASE WHEN json_valid(artifact_retrieval)
                THEN json_type(artifact_retrieval) = 'object'
                    AND json_extract(artifact_retrieval, '$.type') IS NOT NULL
                ELSE 0
            END
        );
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE plugin_marketplace_source DROP COLUMN artifact_retrieval;
"#];

/// Adds the tagged artifact-retrieval configuration while preserving existing sources as HTTPS.
pub fn migration() -> Migration {
    Migration::new("0009", UP_STATEMENTS, DOWN_STATEMENTS)
}
