use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE plugin_marketplace_source
    ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1));
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE plugin_marketplace_source DROP COLUMN enabled;
"#];

/// Records whether a marketplace source participates in sync, listing, and install.
///
/// Disable must keep the row (and its namespace binding) so re-enabling the same URL does not mint
/// a second identity for plugins already installed from it.
pub fn migration() -> Migration {
    Migration::new("0008", UP_STATEMENTS, DOWN_STATEMENTS)
}
