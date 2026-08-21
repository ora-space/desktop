use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <export-contracts | check-dt [--deny-todo] [PATH...]>";

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
        return Err(USAGE.to_string());
    };

    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "failed to determine workspace root".to_string())?;

    match command.as_str() {
        "export-contracts" => {
            if let Some(unexpected) = arguments.next() {
                return Err(format!("unexpected argument `{unexpected}`"));
            }
            xtask::run_export_contracts(workspace_root)
                .map_err(|error| format!("failed to export contracts: {error}"))
        }
        "check-dt" => {
            let args = xtask::CheckDtArgs::parse(arguments)?;
            xtask::run_check_dt(workspace_root, &args)
        }
        other => Err(format!("unknown xtask command `{other}`\n{USAGE}")),
    }
}
