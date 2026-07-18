use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ora_plugin_protocol::{
    ActivateResult, AgentProviderId, AgentRequest, AgentResponse, AgentScope,
    DiscoverInstallationsRequest, DiscoverInstallationsResponse, FrameDecoder, FrameType,
    InitializeParams, InitializeResult, InitializeResultPlugin, JsonRpcEnvelope, JsonSafeU64,
    MAX_FRAME_BYTES, METHOD_ACTIVATE, METHOD_DEACTIVATE, METHOD_EXIT, METHOD_INITIALIZE, PluginId,
    PluginKind, PluginVersion, WIRE_VERSION_V1, encode_frame,
};
use ora_process::{ProcessExit, ProcessSpec, ProcessTreeController, ProcessTreeError};
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::sync::{Notify, mpsc, watch};

use super::{
    AgentInvocationResult, GenerationLaunchError, GenerationLauncher, GenerationProcessEvent,
    GenerationTransport, PluginRuntimeAssets, StderrDrainSummary, WriterQueueLimits, spawn_reader,
    spawn_writer,
};
use crate::{
    LaunchGrantError, LaunchValueReference, LaunchValueResolver, PluginError, PluginManagerConfig,
    PluginRuntimeControl, PluginRuntimeEvent, PluginRuntimeEventSink, ResolvedLaunchValue,
    RuntimeAdmissionProvider, RuntimeState, StopReason, ValidatedLaunchDescriptor,
    spawn_agent_plugin_runtime,
};

#[derive(Clone)]
struct FakeController {
    terminate: watch::Sender<bool>,
}

impl ProcessTreeController for FakeController {
    fn terminate_tree(&self) -> Result<(), ProcessTreeError> {
        let _ = self.terminate.send(true);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct FakeLauncher;

impl GenerationLauncher for FakeLauncher {
    type Controller = FakeController;

    async fn launch(
        &self,
        generation: u64,
        _spec: ProcessSpec,
    ) -> Result<GenerationTransport<Self::Controller>, GenerationLaunchError> {
        fake_generation_transport(generation)
    }
}

/// Builds one in-memory transport whose peer implements the full lifecycle handshake.
fn fake_generation_transport(
    generation: u64,
) -> Result<GenerationTransport<FakeController>, GenerationLaunchError> {
    fake_generation_transport_with_outcome(generation, TreeDrainOutcome::Reaped)
}

/// Builds a lifecycle peer with a caller-selected final process-tree watcher outcome.
fn fake_generation_transport_with_outcome(
    generation: u64,
    tree_outcome: TreeDrainOutcome,
) -> Result<GenerationTransport<FakeController>, GenerationLaunchError> {
    let (host_stdin, child_stdin) = duplex(1024 * 1024);
    let (mut child_stdout, host_stdout) = duplex(1024 * 1024);
    let (writer, writer_events) = spawn_writer(host_stdin, WriterQueueLimits::v1_defaults())
        .map_err(|_| GenerationLaunchError::WriterConfiguration)?;
    let reader_events = spawn_reader(host_stdout, 64, 256);
    let (process_tx, process_events) = mpsc::channel(8);
    let (terminate_tx, terminate_rx) = watch::channel(false);
    tokio::spawn(async move {
        run_fake_bootstrap(
            child_stdin,
            &mut child_stdout,
            process_tx,
            terminate_rx,
            tree_outcome,
        )
        .await;
    });
    Ok(GenerationTransport {
        generation,
        process_id: 4242,
        controller: FakeController {
            terminate: terminate_tx,
        },
        writer,
        writer_events,
        reader_events,
        process_events,
    })
}

#[derive(Clone)]
struct CountingLauncher {
    launches: Arc<AtomicUsize>,
}

impl GenerationLauncher for CountingLauncher {
    type Controller = FakeController;

    async fn launch(
        &self,
        generation: u64,
        _spec: ProcessSpec,
    ) -> Result<GenerationTransport<Self::Controller>, GenerationLaunchError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        fake_generation_transport(generation)
    }
}

#[derive(Clone)]
struct ControlledLauncher {
    launches: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release_launch: Arc<Notify>,
    terminated: Arc<Notify>,
    release_reap: Arc<Notify>,
    tree_outcome: TreeDrainOutcome,
}

impl GenerationLauncher for ControlledLauncher {
    type Controller = FakeController;

    async fn launch(
        &self,
        generation: u64,
        _spec: ProcessSpec,
    ) -> Result<GenerationTransport<Self::Controller>, GenerationLaunchError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release_launch.notified().await;
        controlled_cleanup_transport(
            generation,
            Arc::clone(&self.terminated),
            Arc::clone(&self.release_reap),
            self.tree_outcome,
        )
    }
}

#[derive(Clone, Copy)]
enum TreeDrainOutcome {
    Reaped,
    Failed,
}

#[derive(Clone)]
struct FailingCleanupLauncher {
    launches: Arc<AtomicUsize>,
}

impl GenerationLauncher for FailingCleanupLauncher {
    type Controller = FakeController;

    async fn launch(
        &self,
        generation: u64,
        _spec: ProcessSpec,
    ) -> Result<GenerationTransport<Self::Controller>, GenerationLaunchError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        fake_generation_transport_with_outcome(generation, TreeDrainOutcome::Failed)
    }
}

/// Holds process drain signals behind a test gate so CleanupPending is directly observable.
fn controlled_cleanup_transport(
    generation: u64,
    terminated: Arc<Notify>,
    release_reap: Arc<Notify>,
    tree_outcome: TreeDrainOutcome,
) -> Result<GenerationTransport<FakeController>, GenerationLaunchError> {
    let (host_stdin, child_stdin) = duplex(1024 * 1024);
    let (child_stdout, host_stdout) = duplex(1024 * 1024);
    let (writer, writer_events) = spawn_writer(host_stdin, WriterQueueLimits::v1_defaults())
        .map_err(|_| GenerationLaunchError::WriterConfiguration)?;
    let reader_events = spawn_reader(host_stdout, 64, 256);
    let (process_tx, process_events) = mpsc::channel(8);
    let (terminate_tx, mut terminate_rx) = watch::channel(false);
    tokio::spawn(async move {
        let _ = terminate_rx.changed().await;
        terminated.notify_one();
        release_reap.notified().await;
        drop(child_stdin);
        drop(child_stdout);
        let _ = process_tx
            .send(GenerationProcessEvent::DirectExit(Ok(ProcessExit {
                exit_code: Some(0),
                success: true,
            })))
            .await;
        let _ = process_tx
            .send(GenerationProcessEvent::TreeEmpty(tree_result(tree_outcome)))
            .await;
        let _ = process_tx
            .send(GenerationProcessEvent::StderrDrained(StderrDrainSummary {
                retained: Vec::new(),
                dropped_bytes: 0,
                failure: None,
            }))
            .await;
    });
    Ok(GenerationTransport {
        generation,
        process_id: 4242,
        controller: FakeController {
            terminate: terminate_tx,
        },
        writer,
        writer_events,
        reader_events,
        process_events,
    })
}

/// Maps a closed test outcome to the process-tree watcher result consumed by the runtime.
fn tree_result(outcome: TreeDrainOutcome) -> Result<(), ProcessTreeError> {
    match outcome {
        TreeDrainOutcome::Reaped => Ok(()),
        TreeDrainOutcome::Failed => Err(ProcessTreeError::TreeCleanupTimeout),
    }
}

async fn run_fake_bootstrap(
    mut stdin: DuplexStream,
    stdout: &mut DuplexStream,
    process: mpsc::Sender<GenerationProcessEvent>,
    mut terminate: watch::Receiver<bool>,
    tree_outcome: TreeDrainOutcome,
) {
    let mut decoder =
        FrameDecoder::new(MAX_FRAME_BYTES).unwrap_or_else(|error| panic!("fake decoder: {error}"));
    let mut chunk = vec![0_u8; 4096];
    let mut exit = false;
    while !exit {
        tokio::select! {
            changed = terminate.changed() => {
                let _ = changed;
                break;
            }
            read = stdin.read(&mut chunk) => {
                let bytes = read.unwrap_or_else(|error| panic!("fake stdin read: {error}"));
                if bytes == 0 {
                    break;
                }
                let frames = decoder
                    .decode_chunk(&chunk[..bytes])
                    .unwrap_or_else(|error| panic!("fake frame decode: {error}"));
                for frame in frames {
                    let envelope = ora_plugin_protocol::parse_json_rpc_frame(&frame, 64)
                        .unwrap_or_else(|error| panic!("fake envelope: {error}"));
                    match envelope {
                        JsonRpcEnvelope::Request(request) => {
                            let result = match request.method.as_str() {
                                METHOD_INITIALIZE => {
                                    let params: InitializeParams = serde_json::from_value(
                                        request.params.unwrap_or_else(|| panic!("initialize params")),
                                    )
                                    .unwrap_or_else(|error| panic!("fake initialize params: {error}"));
                                    serde_json::to_value(InitializeResult {
                                        wire_version: WIRE_VERSION_V1,
                                        runtime_version: params.runtime_version,
                                        session_id: params.session_id,
                                        plugin: InitializeResultPlugin {
                                            id: params.plugin.id,
                                            version: params.plugin.version,
                                        },
                                    })
                                    .unwrap_or_else(|error| panic!("fake initialize result: {error}"))
                                }
                                METHOD_ACTIVATE => serde_json::to_value(ActivateResult {
                                    providers: vec![ora_plugin_protocol::DeclaredAgent {
                                        id: AgentProviderId::parse("example").unwrap_or_else(|error| panic!("provider id: {error}")),
                                        contract_version: 1,
                                    }],
                                })
                                .unwrap_or_else(|error| panic!("fake activate result: {error}")),
                                "agent.discoverInstallations" => serde_json::json!({
                                    "installations": [],
                                    "diagnostics": [{
                                        "kind": "notFound",
                                        "message": "No installations found"
                                    }]
                                }),
                                METHOD_DEACTIVATE => serde_json::json!({}),
                                other => panic!("unexpected fake method {other}"),
                            };
                            write_response(stdout, request.id.as_str(), result).await;
                        }
                        JsonRpcEnvelope::Notification(notification) => {
                            if notification.method == METHOD_EXIT {
                                exit = true;
                            }
                        }
                        JsonRpcEnvelope::Response(_) => panic!("Host sent Response to fake bootstrap"),
                    }
                }
            }
        }
    }
    stdout
        .shutdown()
        .await
        .unwrap_or_else(|error| panic!("fake stdout shutdown: {error}"));
    let _ = process
        .send(GenerationProcessEvent::DirectExit(Ok(ProcessExit {
            exit_code: Some(0),
            success: true,
        })))
        .await;
    let _ = process
        .send(GenerationProcessEvent::TreeEmpty(tree_result(tree_outcome)))
        .await;
    let _ = process
        .send(GenerationProcessEvent::StderrDrained(StderrDrainSummary {
            retained: Vec::new(),
            dropped_bytes: 0,
            failure: None,
        }))
        .await;
}

async fn write_response(stdout: &mut DuplexStream, id: &str, result: serde_json::Value) {
    let payload = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .unwrap_or_else(|error| panic!("fake response JSON: {error}"));
    let frame = encode_frame(FrameType::Response, &payload, MAX_FRAME_BYTES)
        .unwrap_or_else(|error| panic!("fake response frame: {error}"));
    stdout
        .write_all(&frame)
        .await
        .unwrap_or_else(|error| panic!("fake response write: {error}"));
}

#[derive(Clone)]
struct FakeAdmission {
    descriptor: ValidatedLaunchDescriptor,
}

impl RuntimeAdmissionProvider for FakeAdmission {
    async fn admit(&self, _plugin_id: &PluginId) -> Result<ValidatedLaunchDescriptor, PluginError> {
        Ok(self.descriptor.clone())
    }

    async fn recheck_after_activate(
        &self,
        _descriptor: &ValidatedLaunchDescriptor,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingEvents {
    events: Mutex<Vec<PluginRuntimeEvent>>,
}

impl PluginRuntimeEventSink for RecordingEvents {
    async fn record(&self, event: PluginRuntimeEvent) -> Result<(), PluginError> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct EmptyResolver;

impl LaunchValueResolver for EmptyResolver {
    async fn resolve(
        &self,
        _reference: &LaunchValueReference,
    ) -> Result<ResolvedLaunchValue, LaunchGrantError> {
        Err(LaunchGrantError::ReferenceUnavailable)
    }
}

/// Exercises the complete single-flight lifecycle around a deterministic fake bootstrap peer.
#[tokio::test]
async fn supervisor_invokes_and_gracefully_reaps_one_generation() {
    let plugin_id =
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("plugin id: {error}"));
    let descriptor = test_descriptor(plugin_id.clone());
    let config = PluginManagerConfig::new(PathBuf::from(r"D:\ora"));
    let assets = PluginRuntimeAssets::new(
        PathBuf::from(r"D:\runtime\bun.exe"),
        PathBuf::from(r"D:\runtime\plugin-host-bootstrap.js"),
        PathBuf::from(r"D:\runtime\empty-bunfig.toml"),
        PluginVersion::parse("1.0.0").unwrap_or_else(|error| panic!("runtime version: {error}")),
    );
    let events = Arc::new(RecordingEvents::default());
    let runtime = spawn_agent_plugin_runtime(
        plugin_id.clone(),
        config,
        assets,
        FakeLauncher,
        Arc::new(FakeAdmission { descriptor }),
        Arc::clone(&events),
        Arc::new(EmptyResolver),
    );
    runtime
        .start()
        .await
        .unwrap_or_else(|error| panic!("explicit start: {error}"));

    let request = AgentRequest::DiscoverInstallations(DiscoverInstallationsRequest {
        provider_id: AgentProviderId::parse("example")
            .unwrap_or_else(|error| panic!("provider id: {error}")),
        scope: AgentScope::Global {},
    });
    let invocation = runtime
        .invoke_with_timeout(request, Duration::from_secs(5))
        .await
        .unwrap_or_else(|error| panic!("invoke: {error}"));
    assert_eq!(
        invocation
            .finish()
            .await
            .unwrap_or_else(|error| panic!("invocation result: {error}")),
        AgentInvocationResult::Response(AgentResponse::DiscoverInstallations(
            DiscoverInstallationsResponse {
                installations: Vec::new(),
                diagnostics: vec![ora_plugin_protocol::AgentDiscoveryDiagnostic {
                    kind: ora_plugin_protocol::AgentDiscoveryDiagnosticKind::NotFound,
                    message: "No installations found".to_owned(),
                }],
            }
        ))
    );
    runtime
        .stop_and_reap(&plugin_id, StopReason::ManualStop)
        .await
        .unwrap_or_else(|error| panic!("stop and reap: {error}"));
    assert_eq!(
        runtime
            .state()
            .await
            .unwrap_or_else(|error| panic!("runtime state: {error}")),
        RuntimeState::Stopped
    );
    assert_eq!(
        events
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|event| match event {
                PluginRuntimeEvent::Started { .. } => "started",
                PluginRuntimeEvent::TreeReaped { .. } => "treeReaped",
                PluginRuntimeEvent::Stopped { .. } => "stopped",
                PluginRuntimeEvent::Crashed { .. } => "crashed",
            })
            .collect::<Vec<_>>(),
        vec!["started", "treeReaped", "stopped"]
    );
}

/// Keeps admission fail-closed when a running generation cannot prove process-tree cleanup.
#[tokio::test]
async fn failed_tree_reap_blocks_restart_and_crash_loop_reset() {
    let plugin_id =
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("plugin id: {error}"));
    let launches = Arc::new(AtomicUsize::new(0));
    let runtime = spawn_agent_plugin_runtime(
        plugin_id.clone(),
        PluginManagerConfig::new(PathBuf::from(r"D:\ora")),
        test_assets(),
        FailingCleanupLauncher {
            launches: Arc::clone(&launches),
        },
        Arc::new(FakeAdmission {
            descriptor: test_descriptor(plugin_id.clone()),
        }),
        Arc::new(RecordingEvents::default()),
        Arc::new(EmptyResolver),
    );
    runtime
        .start()
        .await
        .unwrap_or_else(|error| panic!("explicit start: {error}"));
    let cleanup_error = PluginError::TreeCleanupTimeout {
        plugin_id: plugin_id.clone(),
        generation: 1,
    };
    assert_eq!(
        runtime
            .stop_and_reap(&plugin_id, StopReason::ManualStop)
            .await,
        Err(cleanup_error.clone())
    );
    let blocked = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let state = runtime
                .state()
                .await
                .unwrap_or_else(|error| panic!("runtime state: {error}"));
            if matches!(state, RuntimeState::CleanupPending { .. }) {
                break state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("runtime did not preserve failed cleanup"));
    assert_eq!(
        blocked,
        RuntimeState::CleanupPending {
            generation: 1,
            process_tree: super::ProcessTreeToken(1),
            reason: StopReason::ManualStop,
        }
    );
    assert_eq!(runtime.start().await, Err(cleanup_error.clone()));
    assert_eq!(runtime.reset_crash_loop().await, Ok(()));
    assert_eq!(runtime.start().await, Err(cleanup_error));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

/// Proves every concurrent start joins one launch generation instead of racing new processes.
#[tokio::test]
async fn concurrent_explicit_starts_share_one_generation() {
    let plugin_id =
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("plugin id: {error}"));
    let launches = Arc::new(AtomicUsize::new(0));
    let runtime = spawn_agent_plugin_runtime(
        plugin_id.clone(),
        PluginManagerConfig::new(PathBuf::from(r"D:\ora")),
        test_assets(),
        CountingLauncher {
            launches: Arc::clone(&launches),
        },
        Arc::new(FakeAdmission {
            descriptor: test_descriptor(plugin_id.clone()),
        }),
        Arc::new(RecordingEvents::default()),
        Arc::new(EmptyResolver),
    );
    let mut starts = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let runtime = runtime.clone();
        starts.spawn(async move { runtime.start().await });
    }
    while let Some(start) = starts.join_next().await {
        assert_eq!(
            start.unwrap_or_else(|error| panic!("start task: {error}")),
            Ok(())
        );
    }
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    runtime
        .stop_and_reap(&plugin_id, StopReason::ManualStop)
        .await
        .unwrap_or_else(|error| panic!("stop and reap: {error}"));
}

/// Proves cancellation retains the generation slot through late tree termination and reap.
#[tokio::test]
async fn cancelled_late_launch_reaps_before_settling_or_starting_again() {
    let plugin_id =
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("plugin id: {error}"));
    let launches = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let release_launch = Arc::new(Notify::new());
    let terminated = Arc::new(Notify::new());
    let release_reap = Arc::new(Notify::new());
    let runtime = spawn_agent_plugin_runtime(
        plugin_id.clone(),
        PluginManagerConfig::new(PathBuf::from(r"D:\ora")),
        test_assets(),
        ControlledLauncher {
            launches: Arc::clone(&launches),
            entered: Arc::clone(&entered),
            release_launch: Arc::clone(&release_launch),
            terminated: Arc::clone(&terminated),
            release_reap: Arc::clone(&release_reap),
            tree_outcome: TreeDrainOutcome::Reaped,
        },
        Arc::new(FakeAdmission {
            descriptor: test_descriptor(plugin_id.clone()),
        }),
        Arc::new(RecordingEvents::default()),
        Arc::new(EmptyResolver),
    );
    let first_start = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.start().await }
    });
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap_or_else(|_| panic!("launcher was not entered"));
    let stop = tokio::spawn({
        let runtime = runtime.clone();
        let plugin_id = plugin_id.clone();
        async move { runtime.stop_and_reap(&plugin_id, StopReason::Disable).await }
    });
    let cancelling = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let state = runtime
                .state()
                .await
                .unwrap_or_else(|error| panic!("runtime state: {error}"));
            if matches!(state, RuntimeState::CancellingStart { .. }) {
                break state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("runtime did not enter CancellingStart"));
    assert_eq!(
        cancelling,
        RuntimeState::CancellingStart {
            generation: 1,
            spawn_token: super::SpawnToken(1),
            reason: StopReason::Disable,
        }
    );
    let joined_start = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.start().await }
    });
    tokio::task::yield_now().await;
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    assert!(!first_start.is_finished());
    assert!(!joined_start.is_finished());
    assert!(!stop.is_finished());

    release_launch.notify_one();
    tokio::time::timeout(Duration::from_secs(1), terminated.notified())
        .await
        .unwrap_or_else(|_| panic!("late tree was not terminated"));
    assert_eq!(
        runtime
            .state()
            .await
            .unwrap_or_else(|error| panic!("runtime state: {error}")),
        RuntimeState::CleanupPending {
            generation: 1,
            process_tree: super::ProcessTreeToken(1),
            reason: StopReason::Disable,
        }
    );
    assert!(!first_start.is_finished());
    assert!(!joined_start.is_finished());
    assert!(!stop.is_finished());

    release_reap.notify_one();
    let cancelled = Err(PluginError::Cancelled {
        plugin_id: plugin_id.clone(),
        request_id: "start".to_owned(),
    });
    assert_eq!(
        first_start
            .await
            .unwrap_or_else(|error| panic!("first start task: {error}")),
        cancelled
    );
    assert_eq!(
        joined_start
            .await
            .unwrap_or_else(|error| panic!("joined start task: {error}")),
        Err(PluginError::Cancelled {
            plugin_id: plugin_id.clone(),
            request_id: "start".to_owned(),
        })
    );
    assert_eq!(
        stop.await
            .unwrap_or_else(|error| panic!("stop task: {error}")),
        Ok(())
    );
    assert_eq!(
        runtime
            .state()
            .await
            .unwrap_or_else(|error| panic!("runtime state: {error}")),
        RuntimeState::Stopped
    );
}

/// Keeps a cancelled late spawn permanently blocked when its tree-empty proof fails.
#[tokio::test]
async fn cancelled_late_launch_cleanup_failure_blocks_restart() {
    let plugin_id =
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("plugin id: {error}"));
    let launches = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let release_launch = Arc::new(Notify::new());
    let terminated = Arc::new(Notify::new());
    let release_reap = Arc::new(Notify::new());
    let runtime = spawn_agent_plugin_runtime(
        plugin_id.clone(),
        PluginManagerConfig::new(PathBuf::from(r"D:\ora")),
        test_assets(),
        ControlledLauncher {
            launches: Arc::clone(&launches),
            entered: Arc::clone(&entered),
            release_launch: Arc::clone(&release_launch),
            terminated: Arc::clone(&terminated),
            release_reap: Arc::clone(&release_reap),
            tree_outcome: TreeDrainOutcome::Failed,
        },
        Arc::new(FakeAdmission {
            descriptor: test_descriptor(plugin_id.clone()),
        }),
        Arc::new(RecordingEvents::default()),
        Arc::new(EmptyResolver),
    );
    let start = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.start().await }
    });
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap_or_else(|_| panic!("launcher was not entered"));
    let stop = tokio::spawn({
        let runtime = runtime.clone();
        let plugin_id = plugin_id.clone();
        async move {
            runtime
                .stop_and_reap(&plugin_id, StopReason::Uninstall)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                runtime
                    .state()
                    .await
                    .unwrap_or_else(|error| panic!("runtime state: {error}")),
                RuntimeState::CancellingStart { .. }
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("runtime did not enter CancellingStart"));
    release_launch.notify_one();
    tokio::time::timeout(Duration::from_secs(1), terminated.notified())
        .await
        .unwrap_or_else(|_| panic!("late tree was not terminated"));
    release_reap.notify_one();
    assert_eq!(
        start
            .await
            .unwrap_or_else(|error| panic!("start task: {error}")),
        Err(PluginError::Cancelled {
            plugin_id: plugin_id.clone(),
            request_id: "start".to_owned(),
        })
    );
    let cleanup_error = PluginError::TreeCleanupTimeout {
        plugin_id: plugin_id.clone(),
        generation: 1,
    };
    assert_eq!(
        stop.await
            .unwrap_or_else(|error| panic!("stop task: {error}")),
        Err(cleanup_error.clone())
    );
    assert_eq!(
        runtime
            .state()
            .await
            .unwrap_or_else(|error| panic!("runtime state: {error}")),
        RuntimeState::CleanupPending {
            generation: 1,
            process_tree: super::ProcessTreeToken(1),
            reason: StopReason::Uninstall,
        }
    );
    assert_eq!(runtime.start().await, Err(cleanup_error));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

/// Proves a spawn deadline also owns a late successful tree until cleanup has completed.
#[tokio::test]
async fn timed_out_late_launch_reaps_before_reporting_failure() {
    let plugin_id =
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("plugin id: {error}"));
    let launches = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let release_launch = Arc::new(Notify::new());
    let terminated = Arc::new(Notify::new());
    let release_reap = Arc::new(Notify::new());
    let mut config = PluginManagerConfig::new(PathBuf::from(r"D:\ora"));
    config.deadlines.spawn = Duration::from_millis(10);
    let runtime = spawn_agent_plugin_runtime(
        plugin_id.clone(),
        config,
        test_assets(),
        ControlledLauncher {
            launches: Arc::clone(&launches),
            entered: Arc::clone(&entered),
            release_launch: Arc::clone(&release_launch),
            terminated: Arc::clone(&terminated),
            release_reap: Arc::clone(&release_reap),
            tree_outcome: TreeDrainOutcome::Reaped,
        },
        Arc::new(FakeAdmission {
            descriptor: test_descriptor(plugin_id.clone()),
        }),
        Arc::new(RecordingEvents::default()),
        Arc::new(EmptyResolver),
    );
    let start = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.start().await }
    });
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap_or_else(|_| panic!("launcher was not entered"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    release_launch.notify_one();
    tokio::time::timeout(Duration::from_secs(1), terminated.notified())
        .await
        .unwrap_or_else(|_| panic!("timed-out late tree was not terminated"));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    assert!(!start.is_finished());
    release_reap.notify_one();
    assert_eq!(
        start
            .await
            .unwrap_or_else(|error| panic!("start task: {error}")),
        Err(PluginError::ProcessSpawnFailed {
            plugin_id: plugin_id.clone(),
        })
    );
    assert_eq!(
        runtime
            .state()
            .await
            .unwrap_or_else(|error| panic!("runtime state: {error}")),
        RuntimeState::Stopped
    );
}

/// Returns stable synthetic runtime paths because fake launchers never touch the filesystem.
fn test_assets() -> PluginRuntimeAssets {
    PluginRuntimeAssets::new(
        PathBuf::from(r"D:\runtime\bun.exe"),
        PathBuf::from(r"D:\runtime\plugin-host-bootstrap.js"),
        PathBuf::from(r"D:\runtime\empty-bunfig.toml"),
        PluginVersion::parse("1.0.0").unwrap_or_else(|error| panic!("runtime version: {error}")),
    )
}

fn test_descriptor(plugin_id: PluginId) -> ValidatedLaunchDescriptor {
    ValidatedLaunchDescriptor {
        plugin_id,
        plugin_version: PluginVersion::parse("0.1.0")
            .unwrap_or_else(|error| panic!("plugin version: {error}")),
        kind: PluginKind::Agent,
        content_digest: ora_plugin_protocol::ContentDigest::parse(format!(
            "sha256:{}",
            "a".repeat(64)
        ))
        .unwrap_or_else(|error| panic!("content digest: {error}")),
        content_owner: ora_plugin_protocol::ContentOwnerId::parse(format!(
            "sha256-{}",
            "b".repeat(64)
        ))
        .unwrap_or_else(|error| panic!("content owner: {error}")),
        extension_path: PathBuf::from(r"D:\ora\plugins\ora.runtime"),
        entry_path: PathBuf::from(r"D:\ora\plugins\ora.runtime\dist\index.js"),
        storage_path: PathBuf::from(r"D:\ora\plugin-data\ora.runtime\owner"),
        declared_agents: vec![
            AgentProviderId::parse("example")
                .unwrap_or_else(|error| panic!("provider id: {error}")),
        ],
        enablement_epoch: JsonSafeU64::new(1)
            .unwrap_or_else(|error| panic!("enablement epoch: {error}")),
        registry_revision: JsonSafeU64::new(1)
            .unwrap_or_else(|error| panic!("registry revision: {error}")),
        launch_grant: None,
    }
}
