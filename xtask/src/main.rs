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
        return Err("usage: cargo xtask <export-contracts|export-plugin-sdk>".to_string());
    };

    if let Some(unexpected) = arguments.next() {
        return Err(format!("unexpected argument `{unexpected}`"));
    }

    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "failed to determine workspace root".to_string())?;

    match command.as_str() {
        "export-contracts" => xtask::run_export_contracts(workspace_root)
            .map_err(|error| format!("failed to export contracts: {error}")),
        "export-plugin-sdk" => xtask::run_export_plugin_sdk(workspace_root)
            .map_err(|error| format!("failed to export plugin SDK: {error}")),
        _ => Err(format!("unknown xtask command `{command}`")),
    }
}
