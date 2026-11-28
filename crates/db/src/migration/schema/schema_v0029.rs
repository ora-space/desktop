use super::Migration;

/// Reuses extracted evidence and its digest across interning and reference capture.
pub fn migration() -> Migration {
    Migration::new(
        "0029",
        &[include_str!("workflow_debug_intern.up.sql")],
        &[include_str!("workflow_debug_intern.down.sql")],
    )
}
