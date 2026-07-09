mod json_rpc;

pub use json_rpc::{
    PluginAddParams, PluginJsonRpcError, PluginJsonRpcErrorResponse, PluginJsonRpcRequest,
    PluginJsonRpcSuccessResponse,
};

use std::path::Path;
use ts_rs::{Config, ExportError, TS};

/// Exports plugin protocol DTOs for TypeScript SDK packages that speak the same wire format.
pub fn export_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    let config = Config::new().with_out_dir(output_directory.as_ref());

    PluginAddParams::export(&config)?;
    PluginJsonRpcRequest::export(&config)?;
    PluginJsonRpcSuccessResponse::export(&config)?;
    PluginJsonRpcError::export(&config)?;
    PluginJsonRpcErrorResponse::export(&config)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::export_typescript_bindings_to;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Verifies SDK protocol bindings are written only to the caller-selected package directory.
    #[test]
    fn exports_typescript_protocol_bindings() {
        let output_directory = TempDir::new().unwrap_or_else(|error| {
            panic!("failed to create protocol export directory: {error}");
        });

        export_typescript_bindings_to(output_directory.path()).unwrap_or_else(|error| {
            panic!("expected protocol export to succeed: {error}");
        });

        let generated_source =
            fs::read_to_string(output_directory.path().join("plugin-protocol.ts"))
                .unwrap_or_else(|error| panic!("failed to read protocol export: {error}"));
        let exported_types = generated_source
            .lines()
            .filter(|line| line.starts_with("export type "))
            .collect::<Vec<_>>();

        assert_eq!(
            exported_types,
            vec![
                "export type PluginAddParams = { a: number; b: number };",
                "export type PluginJsonRpcError = { code: number; message: string };",
                "export type PluginJsonRpcErrorResponse = {",
                "export type PluginJsonRpcRequest = {",
                "export type PluginJsonRpcSuccessResponse = {",
            ],
        );
    }
}
