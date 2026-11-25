use super::Migration;

/// Stores opaque, run-owned preview receipts and append-only configuration revisions.
/// Legacy attempts deliberately acquire no inferred entry evidence.
pub fn migration() -> Migration {
    Migration::new(
        "0026",
        &[include_str!("workflow_debug.up.sql")],
        &[include_str!("workflow_debug.down.sql")],
    )
}
