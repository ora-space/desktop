use ora_plugin_manager::{
    AuthorizationHandleFailure, DiscoveryRootId, EffectiveDisableReason, EnvironmentBinding,
    EnvironmentVariableName, LaunchValueReference, ManagementSessionId, PluginError,
    PluginLaunchGrant, PluginManagement, PluginManagementService, PluginManagerConfig,
    PluginRuntimeControl, PluginRuntimeEvent, PluginRuntimeEventSink, RuntimeAdmissionProvider,
    RuntimeSupport, SourceChangeReason, StopReason, SystemAuthorityClock,
};
use ora_plugin_protocol::{JsonSafeU64, PluginId, PluginKind};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use tempfile::TempDir;

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeAction {
    Open(PluginId),
    Close(PluginId),
    Stop(PluginId, StopReason),
    ResetCrashLoop(PluginId),
}

#[derive(Debug, Clone, Default)]
struct RecordingRuntimeControl {
    actions: Arc<Mutex<Vec<RuntimeAction>>>,
}

impl RecordingRuntimeControl {
    /// Returns the complete ordered management-to-runtime call trace.
    fn actions(&self) -> Vec<RuntimeAction> {
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Records one fake runtime action without ever spawning a process.
    fn record(&self, action: RuntimeAction) {
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(action);
    }
}

impl PluginRuntimeControl for RecordingRuntimeControl {
    fn open_admission(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send {
        self.record(RuntimeAction::Open(plugin_id.clone()));
        std::future::ready(Ok(()))
    }

    fn close_admission(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send {
        self.record(RuntimeAction::Close(plugin_id.clone()));
        std::future::ready(Ok(()))
    }

    fn stop_and_reap(
        &self,
        plugin_id: &PluginId,
        reason: StopReason,
    ) -> impl Future<Output = Result<(), PluginError>> + Send {
        self.record(RuntimeAction::Stop(plugin_id.clone(), reason));
        std::future::ready(Ok(()))
    }

    fn reset_crash_loop(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send {
        self.record(RuntimeAction::ResetCrashLoop(plugin_id.clone()));
        std::future::ready(Ok(()))
    }
}

/// Proves the complete management lifecycle remains fail-closed without a process spawner.
#[tokio::test]
async fn scan_install_enable_disable_uninstall_and_restart_without_spawn() {
    let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
    let discovery =
        TempDir::new().unwrap_or_else(|error| panic!("expected discovery root: {error}"));
    let agent_source = discovery.path().join("agent");
    let workbench_source = discovery.path().join("workbench");
    write_agent_fixture(&agent_source, "ora.management-agent");
    write_workbench_fixture(&workbench_source, "ora.management-workbench");

    let root_id = DiscoveryRootId::parse("test-root")
        .unwrap_or_else(|error| panic!("expected discovery id: {error}"));
    let discovery_roots = BTreeMap::from([(root_id.clone(), discovery.path().to_path_buf())]);
    let control = RecordingRuntimeControl::default();
    let service = PluginManagementService::bootstrap(
        PluginManagerConfig::new(data.path()),
        SystemAuthorityClock::new(),
        control.clone(),
        discovery_roots.clone(),
    )
    .await
    .unwrap_or_else(|error| panic!("expected management bootstrap: {error}"));
    let session = ManagementSessionId::new_random()
        .unwrap_or_else(|error| panic!("expected management session: {error}"));

    let selections = service
        .scan_candidates(&session, vec![root_id])
        .await
        .unwrap_or_else(|error| panic!("expected candidate scan: {error}"));
    assert_eq!(selections.len(), 2);
    let mut candidates = BTreeMap::new();
    for selection in selections {
        let identified = service
            .identify(&session, selection.selection_handle)
            .await
            .unwrap_or_else(|error| panic!("expected identify: {error}"));
        candidates.insert(identified.plugin_id, identified.candidate_handle);
    }

    let agent_id = PluginId::parse("ora.management-agent")
        .unwrap_or_else(|error| panic!("expected agent id: {error}"));
    let workbench_id = PluginId::parse("ora.management-workbench")
        .unwrap_or_else(|error| panic!("expected workbench id: {error}"));
    for plugin_id in [&agent_id, &workbench_id] {
        let candidate = candidates
            .remove(plugin_id)
            .unwrap_or_else(|| panic!("expected candidate for {plugin_id}"));
        let installed = service
            .install_authorized_candidate(&session, candidate)
            .await
            .unwrap_or_else(|error| panic!("expected install for {plugin_id}: {error}"));
        assert_eq!(&installed.plugin_id, plugin_id);
    }
    assert!(service.registry_snapshot().await.plugins_by_id.is_empty());

    let catalog = service
        .scan_installed()
        .await
        .unwrap_or_else(|error| panic!("expected installed catalog: {error}"));
    assert_eq!(catalog.entries.len(), 2);
    assert_eq!(
        catalog
            .entries
            .iter()
            .find(|entry| entry.plugin_id.as_ref() == Some(&workbench_id))
            .map(|entry| entry.support.clone()),
        Some(RuntimeSupport::UnsupportedKind {
            kind: PluginKind::Workbench,
        })
    );

    service
        .enable(agent_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected Agent enable: {error}"));
    assert!(
        service
            .registry_snapshot()
            .await
            .plugins_by_id
            .contains_key(&agent_id)
    );
    assert_eq!(
        service.enable(workbench_id.clone()).await,
        Err(PluginError::UnsupportedKind {
            kind: PluginKind::Workbench,
        })
    );

    service
        .disable(agent_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected Agent disable: {error}"));
    assert!(service.registry_snapshot().await.plugins_by_id.is_empty());
    service
        .uninstall(agent_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected Agent uninstall: {error}"));
    service
        .uninstall(workbench_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected Workbench uninstall: {error}"));

    assert_eq!(
        control.actions(),
        vec![
            RuntimeAction::ResetCrashLoop(agent_id.clone()),
            RuntimeAction::Open(agent_id.clone()),
            RuntimeAction::Close(agent_id.clone()),
            RuntimeAction::Stop(agent_id.clone(), StopReason::Disable),
            RuntimeAction::Close(agent_id.clone()),
            RuntimeAction::Stop(agent_id, StopReason::Uninstall),
            RuntimeAction::Close(workbench_id.clone()),
            RuntimeAction::Stop(workbench_id, StopReason::Uninstall),
        ]
    );
    assert_eq!(
        service
            .scan_installed()
            .await
            .unwrap_or_else(|error| panic!("expected empty catalog: {error}"))
            .entries,
        Vec::new()
    );

    drop(service);
    let restarted = PluginManagementService::bootstrap(
        PluginManagerConfig::new(data.path()),
        SystemAuthorityClock::new(),
        RecordingRuntimeControl::default(),
        discovery_roots,
    )
    .await
    .unwrap_or_else(|error| panic!("expected restart bootstrap: {error}"));
    assert_eq!(
        restarted
            .scan_installed()
            .await
            .unwrap_or_else(|error| panic!("expected restart catalog: {error}"))
            .entries,
        Vec::new()
    );
}

/// Proves source changes consume install authority without publishing any final bytes.
#[tokio::test]
async fn source_change_after_identify_fails_closed_and_consumes_candidate() {
    let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
    let source = TempDir::new().unwrap_or_else(|error| panic!("expected source root: {error}"));
    write_agent_fixture(source.path(), "ora.source-change");
    let config = PluginManagerConfig::new(data.path());
    let service = PluginManagementService::bootstrap_without_runtime(config.clone())
        .await
        .unwrap_or_else(|error| panic!("expected management bootstrap: {error}"));
    let session = ManagementSessionId::new_random()
        .unwrap_or_else(|error| panic!("expected management session: {error}"));
    let selection = service
        .register_native_selection(session.clone(), source.path())
        .unwrap_or_else(|error| panic!("expected native selection: {error}"));
    let candidate = service
        .identify(&session, selection.selection_handle)
        .await
        .unwrap_or_else(|error| panic!("expected identify: {error}"))
        .candidate_handle;

    std::fs::write(
        source.path().join("dist").join("index.js"),
        "export default { changed: true };\n",
    )
    .unwrap_or_else(|error| panic!("expected source mutation: {error}"));
    assert_eq!(
        service
            .install_authorized_candidate(&session, candidate.clone())
            .await,
        Err(PluginError::SourceChanged {
            reason: SourceChangeReason::ContentDigestMismatch,
        })
    );
    assert_eq!(
        service
            .install_authorized_candidate(&session, candidate)
            .await,
        Err(PluginError::CandidateHandleInvalid {
            reason: AuthorizationHandleFailure::Unknown,
        })
    );
    assert_eq!(
        service
            .scan_installed()
            .await
            .unwrap_or_else(|error| panic!("expected empty catalog: {error}"))
            .entries,
        Vec::new()
    );
    assert_eq!(
        std::fs::read_dir(config.plugins_dir())
            .unwrap_or_else(|error| panic!("expected plugins directory: {error}"))
            .filter_map(Result::ok)
            .filter(|entry| !matches!(
                entry.file_name().to_string_lossy().as_ref(),
                ".staging" | ".trash"
            ))
            .count(),
        0
    );
}

/// Proves grant revocation and crash-loop blocking survive a management-process restart.
#[tokio::test]
async fn revoked_grant_and_crash_loop_remain_fail_closed_after_restart() {
    let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
    let source = TempDir::new().unwrap_or_else(|error| panic!("expected source root: {error}"));
    write_agent_fixture(source.path(), "ora.management-policy");
    let mut config = PluginManagerConfig::new(data.path());
    config.crash_threshold = 2;
    let control = RecordingRuntimeControl::default();
    let service = PluginManagementService::bootstrap(
        config.clone(),
        SystemAuthorityClock::new(),
        control.clone(),
        BTreeMap::new(),
    )
    .await
    .unwrap_or_else(|error| panic!("expected management bootstrap: {error}"));
    let session = ManagementSessionId::new_random()
        .unwrap_or_else(|error| panic!("expected management session: {error}"));
    let selection = service
        .register_native_selection(session.clone(), source.path())
        .unwrap_or_else(|error| panic!("expected native selection: {error}"));
    let identified = service
        .identify(&session, selection.selection_handle)
        .await
        .unwrap_or_else(|error| panic!("expected identify: {error}"));
    let installed = service
        .install_authorized_candidate(&session, identified.candidate_handle)
        .await
        .unwrap_or_else(|error| panic!("expected install: {error}"));
    let plugin_id = installed.plugin_id;
    service
        .enable(plugin_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected enable: {error}"));

    let grant = PluginLaunchGrant {
        plugin_id: plugin_id.clone(),
        content_owner: installed.record.content_owner.clone(),
        schema_version: 1,
        revision: json_safe(1),
        environment: vec![EnvironmentBinding {
            target: EnvironmentVariableName::parse("ORA_AGENT_TOKEN")
                .unwrap_or_else(|error| panic!("expected environment target: {error}")),
            value: LaunchValueReference::Credential {
                key: "credential-reference".to_owned(),
            },
        }],
    };
    service
        .set_launch_grant(grant.clone())
        .await
        .unwrap_or_else(|error| panic!("expected grant set: {error}"));
    assert_eq!(
        service
            .get_launch_grant(&plugin_id)
            .await
            .unwrap_or_else(|error| panic!("expected grant read: {error}")),
        Some(grant)
    );
    service
        .revoke_launch_grant(plugin_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected grant revoke: {error}"));
    assert_eq!(
        service
            .get_launch_grant(&plugin_id)
            .await
            .unwrap_or_else(|error| panic!("expected revoked grant read: {error}")),
        None
    );

    let events = service.runtime_event_sink();
    for generation in 1..=2 {
        events
            .record(PluginRuntimeEvent::Started {
                plugin_id: plugin_id.clone(),
                content_owner: installed.record.content_owner.clone(),
                generation: json_safe(generation),
                sequence: json_safe(generation * 10),
            })
            .await
            .unwrap_or_else(|error| panic!("expected runtime start event: {error}"));
        events
            .record(PluginRuntimeEvent::Crashed {
                plugin_id: plugin_id.clone(),
                content_owner: installed.record.content_owner.clone(),
                generation: json_safe(generation),
                sequence: json_safe(generation * 10 + 1),
                exit_code: Some(17),
            })
            .await
            .unwrap_or_else(|error| panic!("expected runtime crash event: {error}"));
    }
    service
        .scan_installed()
        .await
        .unwrap_or_else(|error| panic!("expected policy refresh: {error}"));
    assert_eq!(
        service.admit(&plugin_id).await,
        Err(PluginError::Disabled {
            plugin_id: plugin_id.clone(),
            reason: EffectiveDisableReason::CrashLoop,
        })
    );

    drop(events);
    drop(service);
    let restarted_control = RecordingRuntimeControl::default();
    let restarted = PluginManagementService::bootstrap(
        config,
        SystemAuthorityClock::new(),
        restarted_control.clone(),
        BTreeMap::new(),
    )
    .await
    .unwrap_or_else(|error| panic!("expected management restart: {error}"));
    assert_eq!(
        restarted
            .get_launch_grant(&plugin_id)
            .await
            .unwrap_or_else(|error| panic!("expected restart grant read: {error}")),
        None
    );
    assert_eq!(
        restarted.admit(&plugin_id).await,
        Err(PluginError::Disabled {
            plugin_id: plugin_id.clone(),
            reason: EffectiveDisableReason::CrashLoop,
        })
    );
    restarted
        .reset_crash_loop(plugin_id.clone())
        .await
        .unwrap_or_else(|error| panic!("expected crash-loop reset: {error}"));
    restarted
        .admit(&plugin_id)
        .await
        .unwrap_or_else(|error| panic!("expected admission after reset: {error}"));
    assert_eq!(
        restarted_control.actions(),
        vec![
            RuntimeAction::ResetCrashLoop(plugin_id.clone()),
            RuntimeAction::Open(plugin_id.clone()),
        ]
    );
    restarted
        .uninstall(plugin_id)
        .await
        .unwrap_or_else(|error| panic!("expected policy fixture uninstall: {error}"));
}

/// Creates one JSON-safe monotonic identity used only by the typed event fixture.
fn json_safe(value: u64) -> JsonSafeU64 {
    JsonSafeU64::new(value).unwrap_or_else(|error| panic!("expected JSON-safe value: {error}"))
}

/// Writes a minimal valid Agent package without executing any candidate code.
fn write_agent_fixture(root: &Path, plugin_id: &str) {
    std::fs::create_dir_all(root.join("dist"))
        .unwrap_or_else(|error| panic!("expected Agent directory: {error}"));
    std::fs::write(root.join("dist").join("index.js"), "export default {};\n")
        .unwrap_or_else(|error| panic!("expected Agent entry: {error}"));
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{"name":"@ora/management-agent","version":"0.1.0","type":"module","ora":{{"manifestVersion":1,"id":"{plugin_id}","displayName":"Management Agent","kind":"agent","main":"dist/index.js","engines":{{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"}},"contributes":{{"agents":[{{"id":"example","displayName":"Example","contractVersion":1}}]}}}}}}"#
        ),
    )
    .unwrap_or_else(|error| panic!("expected Agent manifest: {error}"));
}

/// Writes a valid catalog-only Workbench package with no executable entry.
fn write_workbench_fixture(root: &Path, plugin_id: &str) {
    std::fs::create_dir_all(root)
        .unwrap_or_else(|error| panic!("expected Workbench directory: {error}"));
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{"name":"@ora/management-workbench","version":"0.1.0","type":"module","ora":{{"manifestVersion":1,"id":"{plugin_id}","displayName":"Management Workbench","kind":"workbench","engines":{{"ora":">=0.1.0 <0.2.0"}},"contributes":{{"workbench":{{"schemaVersion":1}}}}}}}}"#
        ),
    )
    .unwrap_or_else(|error| panic!("expected Workbench manifest: {error}"));
}
