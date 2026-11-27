use super::Migration;

#[cfg(test)]
mod tests;

/// Gives boot recovery a single durable view of the old work protected from ordinary cleanup.
pub fn migration() -> Migration {
    Migration::new(
        "0027",
        &[include_str!("workflow_debug_recovery.sql")],
        &[
            "DROP TRIGGER workflow_debug_end_dispatch; DROP TABLE workflow_debug_dispatch; DROP VIEW workflow_debug_boot_protected;",
        ],
    )
}
