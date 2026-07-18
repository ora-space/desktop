#![cfg(windows)]

mod support;

use ora_plugin_manager::{
    ManagementSessionId, ManagerLease, PluginManagement, PluginManagementService,
    PluginManagerConfig, PluginRuntimeHub, PluginRuntimeInvocation, ProcessTreeGenerationLauncher,
    SystemAuthorityClock, UnavailableLaunchValueResolver,
};
use ora_plugin_protocol::{
    AgentProviderId, AgentRequest, AgentResponse, AgentScope, DiscoverInstallationsRequest,
    DiscoverInstallationsResponse,
};
use ora_process::WindowsJobProcessTreeSpawner;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::Arc;
use support::{pack_agent_fixture, prepare_runtime_assets};
use tempfile::TempDir;

/// Exercises installed management facts through the real pinned Bun bootstrap and Windows Job.
#[tokio::test]
#[ignore = "run after `task prepare-plugin-runtime` to use the verified local runtime cache"]
async fn installed_agent_invokes_through_real_bun_job() {
    let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
    let source = TempDir::new().unwrap_or_else(|error| panic!("expected source root: {error}"));
    let artifact = pack_agent_fixture(source.path(), "ora.e2e");

    let config = PluginManagerConfig::new(data.path());
    let lease = Arc::new(
        ManagerLease::acquire(&config)
            .unwrap_or_else(|error| panic!("expected ManagerLease: {error}")),
    );
    let assets = prepare_runtime_assets(&config).await;
    let runtime = PluginRuntimeHub::new(
        config.clone(),
        assets,
        ProcessTreeGenerationLauncher::new(WindowsJobProcessTreeSpawner::new()),
        UnavailableLaunchValueResolver,
    );
    let management = Arc::new(
        PluginManagementService::bootstrap_with_lease(
            config,
            SystemAuthorityClock::new(),
            runtime.clone(),
            BTreeMap::new(),
            lease,
        )
        .await
        .unwrap_or_else(|error| panic!("expected management bootstrap: {error}")),
    );
    runtime
        .bind(
            Arc::clone(&management),
            Arc::new(management.runtime_event_sink()),
        )
        .unwrap_or_else(|error| panic!("expected runtime binding: {error}"));

    let session = ManagementSessionId::new_random()
        .unwrap_or_else(|error| panic!("expected session: {error}"));
    let selection = management
        .register_native_selection(session.clone(), &artifact)
        .unwrap_or_else(|error| panic!("expected selection: {error}"));
    let identified = management
        .identify(&session, selection.selection_handle)
        .await
        .unwrap_or_else(|error| panic!("expected identify: {error}"));
    let installed = management
        .install_authorized_candidate(&session, identified.candidate_handle)
        .await
        .unwrap_or_else(|error| panic!("expected install: {error}"));
    management
        .enable(installed.plugin_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected enable: {error}"));

    let invocation = runtime
        .invoke(
            &installed.plugin_id,
            AgentRequest::DiscoverInstallations(DiscoverInstallationsRequest {
                provider_id: AgentProviderId::parse("example")
                    .unwrap_or_else(|error| panic!("expected provider: {error}")),
                scope: AgentScope::Global {},
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("expected invoke: {error}"));
    assert_eq!(
        invocation
            .finish()
            .await
            .unwrap_or_else(|error| panic!("expected result: {error}")),
        ora_plugin_manager::AgentInvocationResult::Response(AgentResponse::DiscoverInstallations(
            DiscoverInstallationsResponse {
                installations: Vec::new(),
                diagnostics: vec![ora_plugin_protocol::AgentDiscoveryDiagnostic {
                    kind: ora_plugin_protocol::AgentDiscoveryDiagnosticKind::NotFound,
                    message: "No installations found".to_owned(),
                }],
            }
        ))
    );
    management
        .disable(installed.plugin_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected disable: {error}"));
    management
        .uninstall(installed.plugin_id)
        .await
        .unwrap_or_else(|error| panic!("expected uninstall: {error}"));
}
