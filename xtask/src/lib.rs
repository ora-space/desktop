#[path = "../../apps/desktop/src-tauri/bindings.rs"]
mod desktop_bindings;
mod export_contracts;
mod export_desktop;
mod export_plugin_protocol;
mod frontend;
mod generated_artifacts;
mod reconcile_migrations;
mod rust_architecture;

pub use export_contracts::{check_export_contracts, run_export_contracts};
pub use reconcile_migrations::run_reconcile_migrations;
pub use rust_architecture::{check_rust_architecture, report_rust_architecture};
