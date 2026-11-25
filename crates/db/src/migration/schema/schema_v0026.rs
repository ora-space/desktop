use super::Migration;

/// Gives boot recovery a single durable view of the old work protected from ordinary cleanup.
pub fn migration() -> Migration {
    Migration::new(
        "0026",
        &[include_str!("workflow_debug_recovery.sql")],
        &[
            "DROP TRIGGER workflow_debug_end_dispatch; DROP TABLE workflow_debug_dispatch; DROP VIEW workflow_debug_boot_protected;",
        ],
    )
}
