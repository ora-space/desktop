use std::process::ExitCode;

/// Runs the requested xtask command from the workspace root.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Parses the xtask command line and dispatches to the matching workflow.
fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let Some(command) = arguments.next() else {
        return Err(
            "usage: cargo xtask <export-contracts|check-contracts|check-rust-size|report-rust-size|reconcile-migrations DATA_DIRECTORY>".to_string(),
        );
    };

    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "failed to determine workspace root".to_string())?;

    match command.as_str() {
        "check-rust-size" | "report-rust-size" => {
            if let Some(unexpected) = arguments.next() {
                return Err(format!("unexpected argument `{unexpected}`"));
            }
            let operation = if command == "check-rust-size" {
                xtask::check_rust_architecture
            } else {
                xtask::report_rust_architecture
            };
            operation(workspace_root)
                .map_err(|error| format!("Rust architecture check failed: {error}"))
        }
        "export-contracts" | "check-contracts" => {
            if let Some(unexpected) = arguments.next() {
                return Err(format!("unexpected argument `{unexpected}`"));
            }
            let operation = if command == "check-contracts" {
                xtask::check_export_contracts
            } else {
                xtask::run_export_contracts
            };
            operation(workspace_root)
                .map_err(|error| format!("failed to export contracts: {error}"))
        }
        "reconcile-migrations" => {
            let data_directory = arguments.next().ok_or_else(|| {
                "usage: cargo xtask reconcile-migrations DATA_DIRECTORY".to_string()
            })?;
            if let Some(unexpected) = arguments.next() {
                return Err(format!("unexpected argument `{unexpected}`"));
            }
            xtask::run_reconcile_migrations(std::path::Path::new(&data_directory))
                .map_err(|error| format!("failed to reconcile migrations: {error}"))
        }
        _ => Err(format!("unknown xtask command `{command}`")),
    }
}
