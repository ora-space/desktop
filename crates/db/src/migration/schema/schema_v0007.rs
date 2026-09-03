use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE plugin_source_namespace (
    canonical_url TEXT PRIMARY KEY NOT NULL,
    namespace     TEXT NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL
);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS plugin_source_namespace;
"#];

/// Records the immutable namespace bound to each marketplace source's canonical URL.
///
/// The binding is deliberately a table of its own rather than a column on
/// `plugin_marketplace_source`: a namespace outlives the configuration row it was created for.
/// Once a plugin from a source is installed, that namespace is frozen into the plugin's install
/// path, private data directory, `skills` rows, and Effect Consumer identity, so removing the
/// source and adding it back must reuse the original binding instead of minting a second identity
/// that no installed plugin answers to.
pub fn migration() -> Migration {
    Migration::new("0007", UP_STATEMENTS, DOWN_STATEMENTS)
}
