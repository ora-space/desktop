#![cfg(windows)]

mod support;

use ora_plugin_manager::{
    AgentInvocationResult, EffectiveDisableReason, ManagementSessionId, ManagerLease, PluginError,
    PluginManagement, PluginManagementService, PluginManagerConfig, PluginRuntimeHub,
    PluginRuntimeInvocation, ProcessTreeGenerationLauncher, SystemAuthorityClock,
    UnavailableLaunchValueResolver,
};
use ora_plugin_protocol::{
    AgentConversationId, AgentEvent, AgentFinishReason, AgentInstallationId, AgentOutputChannel,
    AgentPrompt, AgentProviderId, AgentRequest, AgentResponse, AgentScope, AgentTurnId,
    AgentTurnResult, ClientRequestId, DiscoverInstallationsRequest, DiscoverInstallationsResponse,
    PluginId, PluginKind, SendMessageRequest, StartConversationRequest,
};
use ora_process::WindowsJobProcessTreeSpawner;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::Arc;
use support::{pack_agent_fixture, prepare_runtime_assets, write_workbench_fixture};
use tempfile::TempDir;

/// Exercises both public facades through packed SDK code, real Bun, and complete management state.
#[tokio::test]
#[ignore = "run after `task prepare-plugin-runtime` to use the verified local runtime cache"]
async fn complete_management_and_runtime_lifecycle_survives_restart() {
    let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
    let sources = TempDir::new().unwrap_or_else(|error| panic!("expected source root: {error}"));
    let agent_artifact = pack_agent_fixture(&sources.path().join("agent"), "ora.library-e2e");
    let workbench_source = sources.path().join("workbench");
    write_workbench_fixture(&workbench_source, "ora.library-workbench");

    let config = PluginManagerConfig::new(data.path());
    let lease = Arc::new(
        ManagerLease::acquire(&config)
            .unwrap_or_else(|error| panic!("expected ManagerLease: {error}")),
    );
    let runtime = PluginRuntimeHub::new(
        config.clone(),
        prepare_runtime_assets(&config).await,
        ProcessTreeGenerationLauncher::new(WindowsJobProcessTreeSpawner::new()),
        UnavailableLaunchValueResolver,
    );
    let management = Arc::new(
        PluginManagementService::bootstrap_with_lease(
            config.clone(),
            SystemAuthorityClock::new(),
            runtime.clone(),
            BTreeMap::new(),
            Arc::clone(&lease),
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
    let agent_id = install_source(&management, &session, &agent_artifact).await;
    let workbench_id = install_source(&management, &session, &workbench_source).await;
    assert_eq!(agent_id, plugin_id("ora.library-e2e"));
    assert_eq!(workbench_id, plugin_id("ora.library-workbench"));
    assert!(
        management
            .registry_snapshot()
            .await
            .plugins_by_id
            .is_empty()
    );

    assert_eq!(
        runtime.invoke(&agent_id, discover_request()).await.err(),
        Some(PluginError::Disabled {
            plugin_id: agent_id.clone(),
            reason: EffectiveDisableReason::User,
        })
    );
    management
        .enable(agent_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected Agent enable: {error}"));
    assert_eq!(
        management.enable(workbench_id.clone()).await,
        Err(PluginError::UnsupportedKind {
            kind: PluginKind::Workbench,
        })
    );

    let (left, right) = tokio::join!(
        runtime.invoke(&agent_id, discover_request()),
        runtime.invoke(&agent_id, discover_request())
    );
    let (left, right) = (
        left.unwrap_or_else(|error| panic!("expected first invocation: {error}")),
        right.unwrap_or_else(|error| panic!("expected second invocation: {error}")),
    );
    let expected_discovery = AgentInvocationResult::Response(AgentResponse::DiscoverInstallations(
        DiscoverInstallationsResponse {
            installations: Vec::new(),
            diagnostics: vec![ora_plugin_protocol::AgentDiscoveryDiagnostic {
                kind: ora_plugin_protocol::AgentDiscoveryDiagnosticKind::NotFound,
                message: "No installations found".to_owned(),
            }],
        },
    ));
    let (left, right) = tokio::join!(left.finish(), right.finish());
    assert_eq!(
        (
            left.unwrap_or_else(|error| panic!("expected first result: {error}")),
            right.unwrap_or_else(|error| panic!("expected second result: {error}")),
        ),
        (expected_discovery.clone(), expected_discovery)
    );

    let conversation_id = conversation_id();
    let mut stream = runtime
        .invoke(
            &agent_id,
            AgentRequest::StartConversation(StartConversationRequest {
                provider_id: provider_id(),
                installation_id: installation_id(),
                scope: AgentScope::Global {},
                client_request_id: client_request_id("00000000-0000-4000-8000-000000000001"),
                prompt: prompt("hello"),
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("expected streaming invocation: {error}"));
    assert_eq!(
        stream.next_event().await,
        Some(AgentEvent::ConversationStarted {
            conversation_id: conversation_id.clone(),
        })
    );
    assert_eq!(
        stream.next_event().await,
        Some(AgentEvent::TextDelta {
            channel: AgentOutputChannel::Assistant,
            text: "hello".to_owned(),
        })
    );
    assert_eq!(
        stream
            .finish()
            .await
            .unwrap_or_else(|error| panic!("expected stream terminal: {error}")),
        AgentInvocationResult::Turn(AgentTurnResult {
            conversation_id: conversation_id.clone(),
            turn_id: Some(
                AgentTurnId::parse("turn")
                    .unwrap_or_else(|error| panic!("expected turn id: {error}")),
            ),
            finish_reason: AgentFinishReason::Completed,
            usage: None,
        })
    );

    let mut cancelled = runtime
        .invoke(
            &agent_id,
            AgentRequest::SendMessage(SendMessageRequest {
                provider_id: provider_id(),
                installation_id: installation_id(),
                conversation_id: conversation_id.clone(),
                scope: AgentScope::Global {},
                client_request_id: client_request_id("00000000-0000-4000-8000-000000000002"),
                prompt: prompt("cancel me"),
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("expected cancellable invocation: {error}"));
    assert_eq!(
        cancelled.next_event().await,
        Some(AgentEvent::TextDelta {
            channel: AgentOutputChannel::Assistant,
            text: "pending".to_owned(),
        })
    );
    let cancelled_request_id = cancelled.request_id().to_owned();
    cancelled
        .cancel()
        .await
        .unwrap_or_else(|error| panic!("expected transport cancel: {error}"));
    assert_eq!(
        cancelled.finish().await,
        Err(PluginError::Cancelled {
            plugin_id: agent_id.clone(),
            request_id: cancelled_request_id,
        })
    );

    std::fs::write(
        config
            .plugins_dir()
            .join(agent_id.as_str())
            .join("dist")
            .join("index.js"),
        "export default {};\n",
    )
    .unwrap_or_else(|error| panic!("expected installed tamper fixture: {error}"));
    assert_eq!(
        runtime.invoke(&agent_id, discover_request()).await.err(),
        Some(PluginError::Disabled {
            plugin_id: agent_id.clone(),
            reason: EffectiveDisableReason::IntegrityMismatch,
        })
    );

    management
        .disable(agent_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected Agent disable: {error}"));
    assert_eq!(
        runtime.invoke(&agent_id, discover_request()).await.err(),
        Some(PluginError::BackendShuttingDown)
    );
    management
        .uninstall(agent_id)
        .await
        .unwrap_or_else(|error| panic!("expected Agent uninstall: {error}"));
    management
        .uninstall(workbench_id)
        .await
        .unwrap_or_else(|error| panic!("expected Workbench uninstall: {error}"));
    runtime
        .shutdown_all()
        .await
        .unwrap_or_else(|error| panic!("expected global runtime shutdown: {error}"));

    drop(management);
    drop(runtime);
    drop(lease);
    let restarted = PluginManagementService::bootstrap_without_runtime(config)
        .await
        .unwrap_or_else(|error| panic!("expected clean restart: {error}"));
    assert_eq!(
        restarted
            .scan_installed()
            .await
            .unwrap_or_else(|error| panic!("expected restart catalog: {error}"))
            .entries,
        Vec::new()
    );
}

/// Installs a trusted local source exclusively through the two opaque authorization stages.
async fn install_source<Clock, Control>(
    management: &PluginManagementService<Clock, Control>,
    session: &ManagementSessionId,
    source: &std::path::Path,
) -> PluginId
where
    Clock: ora_plugin_manager::AuthorityClock,
    Control: ora_plugin_manager::PluginRuntimeControl,
{
    let selection = management
        .register_native_selection(session.clone(), source)
        .unwrap_or_else(|error| panic!("expected trusted selection: {error}"));
    let identified = management
        .identify(session, selection.selection_handle)
        .await
        .unwrap_or_else(|error| panic!("expected identify: {error}"));
    management
        .install_authorized_candidate(session, identified.candidate_handle)
        .await
        .unwrap_or_else(|error| panic!("expected install: {error}"))
        .plugin_id
}

/// Builds the idempotent request used for concurrent start and integrity rechecks.
fn discover_request() -> AgentRequest {
    AgentRequest::DiscoverInstallations(DiscoverInstallationsRequest {
        provider_id: provider_id(),
        scope: AgentScope::Global {},
    })
}

/// Parses the fixture's canonical plugin identity.
fn plugin_id(value: &str) -> PluginId {
    PluginId::parse(value).unwrap_or_else(|error| panic!("expected plugin id: {error}"))
}

/// Parses the fixture's declared provider identity.
fn provider_id() -> AgentProviderId {
    AgentProviderId::parse("example")
        .unwrap_or_else(|error| panic!("expected provider id: {error}"))
}

/// Parses the fixture's stable external installation identity.
fn installation_id() -> AgentInstallationId {
    AgentInstallationId::parse("installation")
        .unwrap_or_else(|error| panic!("expected installation id: {error}"))
}

/// Parses the conversation identity emitted by the fixture.
fn conversation_id() -> AgentConversationId {
    AgentConversationId::parse("conversation")
        .unwrap_or_else(|error| panic!("expected conversation id: {error}"))
}

/// Parses one canonical UUID-shaped client request identity.
fn client_request_id(value: &str) -> ClientRequestId {
    ClientRequestId::parse(value)
        .unwrap_or_else(|error| panic!("expected client request id: {error}"))
}

/// Validates one bounded prompt used by the streaming fixtures.
fn prompt(value: &str) -> AgentPrompt {
    AgentPrompt::parse(value).unwrap_or_else(|error| panic!("expected prompt: {error}"))
}
