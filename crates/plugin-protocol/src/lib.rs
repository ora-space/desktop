mod agent;
mod fixtures;
mod frame;
mod identity;
mod json_rpc;
mod lifecycle;
mod manifest;
mod strict_json;

pub use agent::*;
pub use fixtures::*;
pub use frame::*;
pub use identity::*;
pub use json_rpc::*;
pub use lifecycle::*;
pub use manifest::*;
pub use strict_json::*;

use std::path::Path;
use ts_rs::{Config, ExportError, TS};

/// Exports author-visible manifest and Agent DTOs to the public SDK package.
pub fn export_public_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    write_typescript_bindings(output_directory.as_ref(), false)
}

/// Exports Host-private lifecycle DTOs consumed by the bundled bootstrap runtime.
pub fn export_runtime_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    write_typescript_bindings(output_directory.as_ref(), true)
}

/// Renders one self-contained module so ts-rs never creates fragile cross-file merge imports.
fn write_typescript_bindings(
    output_directory: &Path,
    include_lifecycle: bool,
) -> Result<(), ExportError> {
    std::fs::create_dir_all(output_directory)?;
    let mut source = String::from(
        "// Generated from ora-plugin-protocol. Do not edit.\n\n\
         export type JsonValue = null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };\n\n",
    );
    let config = Config::new();

    macro_rules! push_declarations {
        ($($type:ty),+ $(,)?) => {
            $(
                source.push_str("export ");
                source.push_str(&<$type>::decl(&config));
                source.push_str("\n\n");
            )+
        };
    }

    push_declarations!(
        PluginId,
        AgentProviderId,
        PluginVersion,
        PluginRelativePath,
        ContentDigest,
        ContentOwnerId,
        OperationId,
        CandidateAuditId,
        AgentProviderKey,
        PluginPackageManifest,
        PackageModuleType,
        PluginManifest,
        PluginKind,
        AgentEngines,
        WorkbenchEngines,
        EngineRange,
        AgentContributions,
        AgentContribution,
        WorkbenchContributions,
        WorkbenchContribution,
        AgentInstallationId,
        AgentConversationId,
        AgentTurnId,
        AgentCursor,
        AgentResourceId,
        AgentToolCallId,
        ProjectHandle,
        WorktreeHandle,
        AgentConfigurationKey,
        ClientRequestId,
        HostResolvedAbsolutePath,
        AgentPrompt,
        Rfc3339Timestamp,
        JsonSafeU64,
        AgentPageLimit,
        FiniteJsonNumber,
        AgentScope,
        DiscoverInstallationsRequest,
        GetConfigurationSummaryRequest,
        ListSkillsRequest,
        ListMcpServersRequest,
        ListConversationsRequest,
        StartConversationRequest,
        SendMessageRequest,
        CancelConversationRequest,
        DiscoverInstallationsResponse,
        AgentInstallation,
        AgentAvailability,
        AgentDiscoveryDiagnostic,
        AgentDiscoveryDiagnosticKind,
        GetConfigurationSummaryResponse,
        AgentConfigurationItem,
        AgentConfigurationValue,
        ListSkillsResponse,
        AgentSkillSummary,
        ListMcpServersResponse,
        AgentMcpServerSummary,
        AgentResourceSource,
        AgentMcpTransport,
        ListConversationsResponse,
        AgentConversationSummary,
        AgentEvent,
        AgentOutputChannel,
        AgentUsage,
        AgentTurnResult,
        AgentFinishReason,
        CancelConversationResponse,
        CancelDisposition,
        AgentMethod,
        InvocationSemantics,
        AgentBusinessFailureKind,
        AgentBusinessErrorData,
    );

    if include_lifecycle {
        push_declarations!(
            InitializeParams,
            InitializePlugin,
            InitializePaths,
            DeclaredAgent,
            InitializeLimits,
            InitializeResult,
            InitializeResultPlugin,
            ActivationReason,
            ActivateParams,
            ActivateResult,
            DeactivationReason,
            DeactivateParams,
            CancelRequestParams,
            StreamParams,
        );
    }

    let trimmed_length = source.trim_end().len();
    source.truncate(trimmed_length);
    source.push('\n');
    std::fs::write(output_directory.join("plugin-protocol.ts"), source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{export_public_typescript_bindings_to, export_runtime_typescript_bindings_to};
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Verifies SDK protocol bindings are written only to the caller-selected package directory.
    #[test]
    fn exports_public_and_private_typescript_bindings() {
        let output_directory = TempDir::new().unwrap_or_else(|error| {
            panic!("failed to create protocol export directory: {error}");
        });

        export_public_typescript_bindings_to(output_directory.path()).unwrap_or_else(|error| {
            panic!("expected protocol export to succeed: {error}");
        });

        let public_files = fs::read_dir(output_directory.path())
            .unwrap_or_else(|error| panic!("failed to read protocol exports: {error}"))
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("failed to read protocol entry: {error}"))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(public_files, vec!["plugin-protocol.ts".to_string()]);
        let public_source = fs::read_to_string(output_directory.path().join("plugin-protocol.ts"))
            .unwrap_or_else(|error| panic!("failed to read public protocol export: {error}"));
        assert!(!public_source.contains("InitializeParams"));
        assert!(public_source.contains("AgentEvent"));
        assert!(!public_source.ends_with("\n\n"));

        let runtime_directory = TempDir::new().unwrap_or_else(|error| {
            panic!("failed to create runtime export directory: {error}");
        });
        export_runtime_typescript_bindings_to(runtime_directory.path()).unwrap_or_else(|error| {
            panic!("expected runtime protocol export to succeed: {error}");
        });
        let runtime_source =
            fs::read_to_string(runtime_directory.path().join("plugin-protocol.ts"))
                .unwrap_or_else(|error| panic!("failed to read runtime protocol export: {error}"));
        assert!(runtime_source.contains("InitializeParams"));
        assert!(!runtime_source.ends_with("\n\n"));
    }
}
