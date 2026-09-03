use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
CREATE TABLE plugin_source_namespace (
    canonical_url TEXT PRIMARY KEY NOT NULL,
    namespace     TEXT NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL
);
"#,
    r#"
ALTER TABLE plugin_marketplace_source
    ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1));
"#,
];

const DOWN_STATEMENTS: &[&str] = &[
    r#"
ALTER TABLE plugin_marketplace_source DROP COLUMN enabled;
"#,
    r#"
DROP TABLE IF EXISTS plugin_source_namespace;
"#,
];

/// Records marketplace source identities and whether each source participates in operations.
///
/// The binding is deliberately a table of its own rather than a column on
/// `plugin_marketplace_source`: a namespace outlives the configuration row it was created for.
/// Once a plugin from a source is installed, that namespace is frozen into the plugin's install
/// path, private data directory, `skills` rows, and Effect Consumer identity, so removing the
/// source and adding it back must reuse the original binding instead of minting a second identity
/// that no installed plugin answers to.
///
/// Disabling a source also keeps its configuration row and namespace binding intact so it can be
/// re-enabled without changing the identity used by installed plugins.
pub fn migration() -> Migration {
    Migration::new("0007", UP_STATEMENTS, DOWN_STATEMENTS)
}
