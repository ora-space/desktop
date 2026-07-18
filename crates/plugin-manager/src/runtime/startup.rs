use std::sync::Arc;

use ora_plugin_protocol::{ActivationReason, JsonSafeU64, PluginId};
use tokio::sync::watch;

use crate::{
    GenerationLaunchError, GenerationLauncher, GenerationProcessEvent, GenerationTransport,
    HandshakeProof, LaunchValueResolver, PluginError, PluginManagerConfig, PluginRuntimeAssets,
    PluginRuntimeEvent, PluginRuntimeEventSink, ReaderEvent, RuntimeAdmissionProvider, StopReason,
    ValidatedLaunchDescriptor, perform_handshake,
};

pub(crate) enum StartWorkerResult<Controller> {
    Ready(Box<StartReady<Controller>>),
    Failed(PluginError),
    CleanupFailed {
        error: PluginError,
        reason: Option<StopReason>,
    },
    Cancelled(Result<(), PluginError>),
}

pub(crate) struct StartReady<Controller> {
    pub(crate) descriptor: ValidatedLaunchDescriptor,
    pub(crate) transport: GenerationTransport<Controller>,
    pub(crate) proof: HandshakeProof,
}

pub(crate) struct StartWorkerContext<Launcher, Admission, Events, Resolver> {
    pub(crate) plugin_id: PluginId,
    pub(crate) generation: u64,
    pub(crate) config: PluginManagerConfig,
    pub(crate) assets: PluginRuntimeAssets,
    pub(crate) launcher: Launcher,
    pub(crate) admission: Arc<Admission>,
    pub(crate) events: Arc<Events>,
    pub(crate) resolver: Arc<Resolver>,
    pub(crate) cleanup: watch::Sender<Option<StopReason>>,
}

enum LaunchRace<Controller> {
    Completed(Result<GenerationTransport<Controller>, GenerationLaunchError>),
    Cancelled(StopReason),
    TimedOut,
}

/// Owns startup through late spawn completion so a cancelled worker cannot leak a concurrent tree.
pub(crate) async fn run_start_worker<Launcher, Admission, Events, Resolver>(
    context: StartWorkerContext<Launcher, Admission, Events, Resolver>,
    mut cancel: watch::Receiver<Option<StopReason>>,
) -> StartWorkerResult<Launcher::Controller>
where
    Launcher: GenerationLauncher,
    Admission: RuntimeAdmissionProvider + Send + Sync + 'static,
    Events: PluginRuntimeEventSink + Send + Sync + 'static,
    Resolver: LaunchValueResolver + Send + Sync + 'static,
{
    let StartWorkerContext {
        plugin_id,
        generation,
        config,
        assets,
        launcher,
        admission,
        events,
        resolver,
        cleanup,
    } = context;
    let descriptor = tokio::select! {
        descriptor = admission.admit(&plugin_id) => match descriptor {
            Ok(descriptor) => descriptor,
            Err(error) => return StartWorkerResult::Failed(error),
        },
        changed = cancel.changed() => {
            let _ = changed;
            return StartWorkerResult::Cancelled(Ok(()));
        }
    };
    let assets = match assets.verified_for_spawn().await {
        Ok(assets) => assets,
        Err(error) => return StartWorkerResult::Failed(error),
    };
    let spec = match assets.process_spec(&descriptor, resolver.as_ref()).await {
        Ok(spec) => spec,
        Err(error) => return StartWorkerResult::Failed(error),
    };
    if stop_reason(&cancel).is_some() {
        return StartWorkerResult::Cancelled(Ok(()));
    }

    // The join handle is deliberately retained after the race. Dropping the future cannot cancel
    // spawn_blocking, so the late result must remain owned until its tree is either absent or reaped.
    let launch_task = tokio::spawn(async move { launcher.launch(generation, spec).await });
    tokio::pin!(launch_task);
    let spawn_deadline = tokio::time::sleep(config.deadlines.spawn);
    tokio::pin!(spawn_deadline);
    let launch_race = tokio::select! {
        result = &mut launch_task => LaunchRace::Completed(flatten_launch_result(result)),
        changed = cancel.changed() => {
            let _ = changed;
            LaunchRace::Cancelled(stop_reason(&cancel).unwrap_or(StopReason::ManualStop))
        }
        () = &mut spawn_deadline => LaunchRace::TimedOut,
    };
    let mut transport = match launch_race {
        LaunchRace::Completed(Ok(transport)) => transport,
        LaunchRace::Completed(Err(_)) => {
            return StartWorkerResult::Failed(PluginError::ProcessSpawnFailed { plugin_id });
        }
        LaunchRace::Cancelled(reason) => {
            let cleanup_result = cleanup_late_launch(
                &plugin_id,
                &config,
                flatten_launch_result(launch_task.await),
                &cleanup,
                reason,
            )
            .await;
            return StartWorkerResult::Cancelled(cleanup_result);
        }
        LaunchRace::TimedOut => {
            let cleanup_result = cleanup_late_launch_after_timeout(
                &plugin_id,
                &config,
                flatten_launch_result(launch_task.await),
            )
            .await;
            return match cleanup_result {
                Ok(()) => StartWorkerResult::Failed(PluginError::ProcessSpawnFailed { plugin_id }),
                Err(error) => StartWorkerResult::CleanupFailed {
                    error,
                    reason: None,
                },
            };
        }
    };
    if let Some(reason) = stop_reason(&cancel) {
        let _ = cleanup.send(Some(reason));
        let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
        return StartWorkerResult::Cancelled(cleanup_result);
    }
    let handshake = tokio::select! {
        handshake = perform_handshake(
            &mut transport,
            admission.as_ref(),
            &descriptor,
            &config,
            &assets,
            ActivationReason::LazyInvocation,
        ) => handshake,
        changed = cancel.changed() => {
            let _ = changed;
            let reason = stop_reason(&cancel).unwrap_or(StopReason::ManualStop);
            let _ = cleanup.send(Some(reason));
            let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
            return StartWorkerResult::Cancelled(cleanup_result);
        }
    };
    let proof = match handshake {
        Ok(proof) => proof,
        Err(error) => {
            let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
            return failed_after_cleanup(error, cleanup_result);
        }
    };
    if let Some(reason) = stop_reason(&cancel) {
        let _ = cleanup.send(Some(reason));
        let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
        return StartWorkerResult::Cancelled(cleanup_result);
    }
    let generation_value = match json_safe(generation) {
        Ok(value) => value,
        Err(error) => {
            let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
            return failed_after_cleanup(error, cleanup_result);
        }
    };
    let sequence_value = match json_safe(event_sequence(generation, 1)) {
        Ok(value) => value,
        Err(error) => {
            let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
            return failed_after_cleanup(error, cleanup_result);
        }
    };
    if let Err(error) = events
        .record(PluginRuntimeEvent::Started {
            plugin_id: plugin_id.clone(),
            content_owner: descriptor.content_owner.clone(),
            generation: generation_value,
            sequence: sequence_value,
        })
        .await
    {
        let cleanup_result = cleanup_start_transport(&plugin_id, &config, transport).await;
        return failed_after_cleanup(error, cleanup_result);
    }

    StartWorkerResult::Ready(Box::new(StartReady {
        descriptor,
        transport,
        proof,
    }))
}

fn flatten_launch_result<Controller>(
    result: Result<
        Result<GenerationTransport<Controller>, GenerationLaunchError>,
        tokio::task::JoinError,
    >,
) -> Result<GenerationTransport<Controller>, GenerationLaunchError> {
    result.unwrap_or(Err(GenerationLaunchError::SpawnWorkerFailed))
}

/// Copies the current cancellation reason without retaining a watch guard across await points.
fn stop_reason(cancel: &watch::Receiver<Option<StopReason>>) -> Option<StopReason> {
    *cancel.borrow()
}

/// Preserves cleanup failure as a fail-closed supervisor phase instead of hiding it behind startup.
fn failed_after_cleanup<Controller>(
    original: PluginError,
    cleanup: Result<(), PluginError>,
) -> StartWorkerResult<Controller> {
    match cleanup {
        Ok(()) => StartWorkerResult::Failed(original),
        Err(error) => StartWorkerResult::CleanupFailed {
            error,
            reason: None,
        },
    }
}

/// Reaps a late successful launch and records the observable cleanup phase for explicit stops.
async fn cleanup_late_launch<Controller>(
    plugin_id: &PluginId,
    config: &PluginManagerConfig,
    result: Result<GenerationTransport<Controller>, GenerationLaunchError>,
    cleanup: &watch::Sender<Option<StopReason>>,
    reason: StopReason,
) -> Result<(), PluginError>
where
    Controller: ora_process::ProcessTreeController,
{
    match result {
        Ok(transport) => {
            let _ = cleanup.send(Some(reason));
            cleanup_start_transport(plugin_id, config, transport).await
        }
        Err(_) => Ok(()),
    }
}

/// Reaps a launch that completed after its deadline before reporting the spawn failure.
async fn cleanup_late_launch_after_timeout<Controller>(
    plugin_id: &PluginId,
    config: &PluginManagerConfig,
    result: Result<GenerationTransport<Controller>, GenerationLaunchError>,
) -> Result<(), PluginError>
where
    Controller: ora_process::ProcessTreeController,
{
    match result {
        Ok(transport) => cleanup_start_transport(plugin_id, config, transport).await,
        Err(_) => Ok(()),
    }
}

/// Terminates and drains every resource created before a generation becomes supervisor-owned.
async fn cleanup_start_transport<Controller>(
    plugin_id: &PluginId,
    config: &PluginManagerConfig,
    mut transport: GenerationTransport<Controller>,
) -> Result<(), PluginError>
where
    Controller: ora_process::ProcessTreeController,
{
    let _ = transport.controller.terminate_tree();
    drop(transport.writer);
    let deadline = config.deadlines.tree_cleanup + config.deadlines.pipe_drain;
    tokio::time::timeout(deadline, async {
        let mut stdout = false;
        let mut stderr = false;
        let mut direct = false;
        let mut tree = false;
        while !(stdout && stderr && direct && tree) {
            tokio::select! {
                reader = transport.reader_events.recv(), if !stdout => {
                    match reader {
                        Some(ReaderEvent::BoundaryEof | ReaderEvent::Failure(_)) | None => stdout = true,
                        Some(ReaderEvent::Envelope(_)) => {}
                    }
                }
                process = transport.process_events.recv() => {
                    match process {
                        Some(GenerationProcessEvent::DirectExit(_)) => direct = true,
                        Some(GenerationProcessEvent::TreeEmpty(result)) => {
                            result.map_err(|_| PluginError::TreeCleanupTimeout {
                                plugin_id: plugin_id.clone(),
                                generation: transport.generation,
                            })?;
                            tree = true;
                        }
                        Some(GenerationProcessEvent::StderrDrained(_)) => stderr = true,
                        None => return Err(PluginError::TreeCleanupTimeout {
                            plugin_id: plugin_id.clone(),
                            generation: transport.generation,
                        }),
                    }
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| PluginError::TreeCleanupTimeout {
        plugin_id: plugin_id.clone(),
        generation: transport.generation,
    })?
}

pub(crate) fn event_sequence(generation: u64, offset: u64) -> u64 {
    generation.saturating_mul(4).saturating_sub(4) + offset
}

pub(crate) fn json_safe(value: u64) -> Result<JsonSafeU64, PluginError> {
    JsonSafeU64::new(value).map_err(|error| PluginError::Internal {
        message: error.to_string(),
    })
}
