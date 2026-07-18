use std::collections::VecDeque;
use std::sync::Arc;

use ora_plugin_protocol::{AgentRequest, PluginId};
use ora_process::ProcessTreeController;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use crate::{
    GenerationCommand, GenerationExit, GenerationLauncher, LaunchValueResolver, PluginError,
    PluginManagerConfig, PluginRuntimeAssets, PluginRuntimeControl, PluginRuntimeEvent,
    PluginRuntimeEventSink, ProcessTreeToken, QueuedInvocation, RuntimeAdmissionProvider,
    RuntimeState, SpawnToken, StartReady, StartWorkerContext, StartWorkerResult, StopReason,
    ValidatedLaunchDescriptor, event_sequence, json_safe, run_start_worker, spawn_generation_actor,
};

/// Cloneable per-plugin facade; the background supervisor remains the only lifecycle owner.
#[derive(Clone)]
pub struct AgentPluginRuntime<Controller> {
    plugin_id: PluginId,
    commands: mpsc::Sender<SupervisorCommand<Controller>>,
    invocation_timeout: std::time::Duration,
}

impl<Controller> AgentPluginRuntime<Controller>
where
    Controller: ora_process::ProcessTreeController,
{
    /// Starts or joins the lazy single-flight generation without issuing a business request.
    pub async fn start(&self) -> Result<(), PluginError> {
        let (completed_tx, completed_rx) = oneshot::channel();
        self.commands
            .send(SupervisorCommand::Start(completed_tx))
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        completed_rx
            .await
            .unwrap_or(Err(PluginError::BackendShuttingDown))
    }

    /// Starts or joins the single generation and returns only after the request is accepted there.
    pub async fn invoke(
        &self,
        request: AgentRequest,
    ) -> Result<crate::AgentInvocationHandle, PluginError> {
        self.invoke_with_timeout(request, self.invocation_timeout)
            .await
    }

    /// Uses one absolute outcome deadline spanning start, admission, queueing, write, and response.
    pub async fn invoke_with_timeout(
        &self,
        request: AgentRequest,
        timeout: std::time::Duration,
    ) -> Result<crate::AgentInvocationHandle, PluginError> {
        let (accepted_tx, accepted_rx) = oneshot::channel();
        self.commands
            .send(SupervisorCommand::Invoke(Box::new(QueuedInvocation {
                request,
                deadline: Instant::now() + timeout,
                accepted: accepted_tx,
            })))
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        accepted_rx
            .await
            .unwrap_or(Err(PluginError::BackendShuttingDown))
    }

    /// Returns an immutable phase projection from the single supervisor owner.
    pub async fn state(&self) -> Result<RuntimeState, PluginError> {
        let (state_tx, state_rx) = oneshot::channel();
        self.commands
            .send(SupervisorCommand::Inspect(state_tx))
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        state_rx.await.map_err(|_| PluginError::BackendShuttingDown)
    }

    /// Clears a durable crash-loop gate after the management mutation has authorized reset.
    pub async fn reset_crash_loop(&self) -> Result<(), PluginError> {
        let (completed_tx, completed_rx) = oneshot::channel();
        self.commands
            .send(SupervisorCommand::ResetCrashLoop(completed_tx))
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        completed_rx
            .await
            .unwrap_or(Err(PluginError::BackendShuttingDown))
    }
}

impl<Controller> PluginRuntimeControl for AgentPluginRuntime<Controller>
where
    Controller: ora_process::ProcessTreeController,
{
    async fn open_admission(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        self.require_plugin(plugin_id)
    }

    async fn close_admission(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        self.require_plugin(plugin_id)?;
        let (completed_tx, completed_rx) = oneshot::channel();
        self.commands
            .send(SupervisorCommand::CloseAdmission(completed_tx))
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        completed_rx
            .await
            .unwrap_or(Err(PluginError::BackendShuttingDown))
    }

    async fn stop_and_reap(
        &self,
        plugin_id: &PluginId,
        reason: StopReason,
    ) -> Result<(), PluginError> {
        self.require_plugin(plugin_id)?;
        let (completed_tx, completed_rx) = oneshot::channel();
        self.commands
            .send(SupervisorCommand::Stop {
                reason,
                completed: completed_tx,
            })
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        completed_rx
            .await
            .unwrap_or(Err(PluginError::BackendShuttingDown))
    }

    async fn reset_crash_loop(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        self.require_plugin(plugin_id)?;
        AgentPluginRuntime::reset_crash_loop(self).await
    }
}

impl<Controller> AgentPluginRuntime<Controller> {
    fn require_plugin(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        if plugin_id != &self.plugin_id {
            return Err(PluginError::NotFound {
                plugin_id: plugin_id.clone(),
            });
        }
        Ok(())
    }
}

/// Creates a single-flight runtime supervisor using statically dispatched production boundaries.
pub fn spawn_agent_plugin_runtime<Launcher, Admission, Events, Resolver>(
    plugin_id: PluginId,
    config: PluginManagerConfig,
    assets: PluginRuntimeAssets,
    launcher: Launcher,
    admission: Arc<Admission>,
    events: Arc<Events>,
    resolver: Arc<Resolver>,
) -> AgentPluginRuntime<Launcher::Controller>
where
    Launcher: GenerationLauncher,
    Admission: RuntimeAdmissionProvider + Send + Sync + 'static,
    Events: PluginRuntimeEventSink + Send + Sync + 'static,
    Resolver: LaunchValueResolver + Send + Sync + 'static,
{
    let (commands_tx, commands_rx) = mpsc::channel(256);
    let runtime = AgentPluginRuntime {
        plugin_id: plugin_id.clone(),
        commands: commands_tx.clone(),
        invocation_timeout: config.deadlines.invocation,
    };
    let supervisor = RuntimeSupervisor {
        plugin_id,
        config,
        assets,
        launcher,
        admission,
        events,
        resolver,
        commands_tx,
        commands_rx,
        phase: SupervisorPhase::Stopped,
        next_generation: 1,
        recent_crashes: VecDeque::new(),
        start_backoff_until: None,
    };
    tokio::spawn(supervisor.run());
    runtime
}

enum SupervisorCommand<Controller> {
    Start(oneshot::Sender<Result<(), PluginError>>),
    Invoke(Box<QueuedInvocation>),
    CloseAdmission(oneshot::Sender<Result<(), PluginError>>),
    Stop {
        reason: StopReason,
        completed: oneshot::Sender<Result<(), PluginError>>,
    },
    Inspect(oneshot::Sender<RuntimeState>),
    ResetCrashLoop(oneshot::Sender<Result<(), PluginError>>),
    StartCompleted {
        generation: u64,
        result: StartWorkerResult<Controller>,
    },
    GenerationCompleted {
        generation: u64,
        exit: GenerationExit,
        descriptor: Box<ValidatedLaunchDescriptor>,
    },
}

enum SupervisorPhase {
    Stopped,
    Starting {
        generation: u64,
        spawn_token: SpawnToken,
        cancel: watch::Sender<Option<StopReason>>,
        cleanup: watch::Receiver<Option<StopReason>>,
        queued: Vec<QueuedInvocation>,
        start_waiters: Vec<oneshot::Sender<Result<(), PluginError>>>,
        stop_waiters: Vec<(StopReason, oneshot::Sender<Result<(), PluginError>>)>,
    },
    Running {
        generation: u64,
        process_id: u32,
        commands: mpsc::Sender<GenerationCommand>,
    },
    CrashLoop {
        recent_crashes: u32,
    },
    CleanupBlocked {
        generation: u64,
        reason: Option<StopReason>,
        error: PluginError,
    },
}

struct RuntimeSupervisor<Launcher, Admission, Events, Resolver>
where
    Launcher: GenerationLauncher,
{
    plugin_id: PluginId,
    config: PluginManagerConfig,
    assets: PluginRuntimeAssets,
    launcher: Launcher,
    admission: Arc<Admission>,
    events: Arc<Events>,
    resolver: Arc<Resolver>,
    commands_tx: mpsc::Sender<SupervisorCommand<Launcher::Controller>>,
    commands_rx: mpsc::Receiver<SupervisorCommand<Launcher::Controller>>,
    phase: SupervisorPhase,
    next_generation: u64,
    recent_crashes: VecDeque<Instant>,
    start_backoff_until: Option<Instant>,
}

impl<Launcher, Admission, Events, Resolver> RuntimeSupervisor<Launcher, Admission, Events, Resolver>
where
    Launcher: GenerationLauncher,
    Admission: RuntimeAdmissionProvider + Send + Sync + 'static,
    Events: PluginRuntimeEventSink + Send + Sync + 'static,
    Resolver: LaunchValueResolver + Send + Sync + 'static,
{
    async fn run(mut self) {
        while let Some(command) = self.commands_rx.recv().await {
            match command {
                SupervisorCommand::Start(completed) => self.handle_start(completed),
                SupervisorCommand::Invoke(invocation) => self.handle_invoke(*invocation),
                SupervisorCommand::CloseAdmission(completed) => self.close_admission(completed),
                SupervisorCommand::Stop { reason, completed } => self.stop(reason, completed),
                SupervisorCommand::Inspect(completed) => {
                    let _ = completed.send(self.state_projection());
                }
                SupervisorCommand::ResetCrashLoop(completed) => {
                    self.recent_crashes.clear();
                    self.start_backoff_until = None;
                    if matches!(self.phase, SupervisorPhase::CrashLoop { .. }) {
                        self.phase = SupervisorPhase::Stopped;
                    }
                    let _ = completed.send(Ok(()));
                }
                SupervisorCommand::StartCompleted { generation, result } => {
                    self.complete_start(generation, result).await;
                }
                SupervisorCommand::GenerationCompleted {
                    generation,
                    exit,
                    descriptor,
                } => {
                    self.complete_generation(generation, exit, *descriptor)
                        .await;
                }
            }
        }
    }

    fn handle_invoke(&mut self, invocation: QueuedInvocation) {
        match &mut self.phase {
            SupervisorPhase::Stopped => {
                if self
                    .start_backoff_until
                    .is_some_and(|deadline| Instant::now() < deadline)
                {
                    let _ = invocation
                        .accepted
                        .send(Err(PluginError::PluginRuntimeUnavailable));
                    return;
                }
                self.begin_start(StartDemand::Invocation(Box::new(invocation)));
            }
            SupervisorPhase::Starting { queued, .. } => {
                if queued.len() >= self.config.limits.runtime.max_pending_requests as usize {
                    let _ = invocation
                        .accepted
                        .send(Err(PluginError::PluginRuntimeUnavailable));
                } else {
                    queued.push(invocation);
                }
            }
            SupervisorPhase::Running { commands, .. } => {
                if let Err(error) = commands.try_send(GenerationCommand::Invoke(invocation))
                    && let GenerationCommand::Invoke(invocation) = error.into_inner()
                {
                    let _ = invocation
                        .accepted
                        .send(Err(PluginError::PluginRuntimeUnavailable));
                }
            }
            SupervisorPhase::CrashLoop { .. } => {
                let _ = invocation
                    .accepted
                    .send(Err(PluginError::PluginRuntimeUnavailable));
            }
            SupervisorPhase::CleanupBlocked { error, .. } => {
                let _ = invocation.accepted.send(Err(error.clone()));
            }
        }
    }

    fn handle_start(&mut self, completed: oneshot::Sender<Result<(), PluginError>>) {
        match &mut self.phase {
            SupervisorPhase::Stopped => self.begin_start(StartDemand::Explicit(completed)),
            SupervisorPhase::Starting { start_waiters, .. } => start_waiters.push(completed),
            SupervisorPhase::Running { .. } => {
                let _ = completed.send(Ok(()));
            }
            SupervisorPhase::CrashLoop { .. } => {
                let _ = completed.send(Err(PluginError::PluginRuntimeUnavailable));
            }
            SupervisorPhase::CleanupBlocked { error, .. } => {
                let _ = completed.send(Err(error.clone()));
            }
        }
    }

    fn begin_start(&mut self, demand: StartDemand) {
        let generation = self.next_generation;
        let next_generation = generation.checked_add(1);
        if next_generation.is_none() || json_safe(event_sequence(generation, 3)).is_err() {
            demand.fail(PluginError::Internal {
                message: "runtime generation identity space is exhausted".to_owned(),
            });
            return;
        }
        self.next_generation = next_generation.unwrap_or(generation);
        let (queued, start_waiters) = demand.into_queues();
        let (cancel_tx, cancel_rx) = watch::channel(None);
        let (cleanup_tx, cleanup_rx) = watch::channel(None);
        let spawn_token = SpawnToken(generation);
        self.phase = SupervisorPhase::Starting {
            generation,
            spawn_token,
            cancel: cancel_tx,
            cleanup: cleanup_rx,
            queued,
            start_waiters,
            stop_waiters: Vec::new(),
        };

        let commands = self.commands_tx.clone();
        let plugin_id = self.plugin_id.clone();
        let config = self.config.clone();
        let assets = self.assets.clone();
        let launcher = self.launcher.clone();
        let admission = Arc::clone(&self.admission);
        let events = Arc::clone(&self.events);
        let resolver = Arc::clone(&self.resolver);
        tokio::spawn(async move {
            let result = run_start_worker(
                StartWorkerContext {
                    plugin_id,
                    generation,
                    config,
                    assets,
                    launcher,
                    admission,
                    events,
                    resolver,
                    cleanup: cleanup_tx,
                },
                cancel_rx,
            )
            .await;
            let _ = commands
                .send(SupervisorCommand::StartCompleted { generation, result })
                .await;
        });
    }

    fn close_admission(&mut self, completed: oneshot::Sender<Result<(), PluginError>>) {
        match &mut self.phase {
            SupervisorPhase::Stopped
            | SupervisorPhase::CrashLoop { .. }
            | SupervisorPhase::CleanupBlocked { .. } => {
                let _ = completed.send(Ok(()));
            }
            SupervisorPhase::Starting { cancel, .. } => {
                let _ = cancel.send(Some(StopReason::ManualStop));
                let _ = completed.send(Ok(()));
            }
            SupervisorPhase::Running { commands, .. } => {
                if let Err(error) = commands.try_send(GenerationCommand::CloseAdmission(completed))
                    && let GenerationCommand::CloseAdmission(completed) = error.into_inner()
                {
                    let _ = completed.send(Err(PluginError::PluginRuntimeUnavailable));
                }
            }
        }
    }

    fn stop(&mut self, reason: StopReason, completed: oneshot::Sender<Result<(), PluginError>>) {
        match &mut self.phase {
            SupervisorPhase::Stopped | SupervisorPhase::CrashLoop { .. } => {
                let _ = completed.send(Ok(()));
            }
            SupervisorPhase::CleanupBlocked { error, .. } => {
                let _ = completed.send(Err(error.clone()));
            }
            SupervisorPhase::Starting {
                cancel,
                stop_waiters,
                ..
            } => {
                let _ = cancel.send(Some(reason));
                stop_waiters.push((reason, completed));
            }
            SupervisorPhase::Running { commands, .. } => {
                if let Err(error) = commands.try_send(GenerationCommand::Stop { reason, completed })
                    && let GenerationCommand::Stop { completed, .. } = error.into_inner()
                {
                    let _ = completed.send(Err(PluginError::PluginRuntimeUnavailable));
                }
            }
        }
    }

    async fn complete_start(
        &mut self,
        generation: u64,
        result: StartWorkerResult<Launcher::Controller>,
    ) {
        let phase = std::mem::replace(&mut self.phase, SupervisorPhase::Stopped);
        let SupervisorPhase::Starting {
            generation: expected,
            cancel,
            queued,
            start_waiters,
            stop_waiters,
            ..
        } = phase
        else {
            if let StartWorkerResult::Ready(ready) = result {
                let _ = ready.transport.controller.terminate_tree();
            }
            return;
        };
        if generation != expected {
            return;
        }

        match result {
            StartWorkerResult::Ready(ready) => {
                let StartReady {
                    descriptor,
                    transport,
                    proof,
                } = *ready;
                let process_id = transport.process_id;
                let actor = spawn_generation_actor(
                    descriptor.clone(),
                    transport,
                    proof,
                    self.config.limits.clone(),
                    self.config.deadlines.clone(),
                );
                for invocation in queued {
                    if actor
                        .commands
                        .send(GenerationCommand::Invoke(invocation))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                for (reason, waiter) in stop_waiters {
                    let _ = actor
                        .commands
                        .send(GenerationCommand::Stop {
                            reason,
                            completed: waiter,
                        })
                        .await;
                }
                let commands = self.commands_tx.clone();
                tokio::spawn(async move {
                    let exit = actor.completed.await.unwrap_or(GenerationExit {
                        expected: false,
                        exit_code: None,
                        cleanup_result: Err(PluginError::PluginRuntimeUnavailable),
                        stop_reason: None,
                    });
                    let _ = commands
                        .send(SupervisorCommand::GenerationCompleted {
                            generation,
                            exit,
                            descriptor: Box::new(descriptor),
                        })
                        .await;
                });
                self.phase = SupervisorPhase::Running {
                    generation,
                    process_id,
                    commands: actor.commands,
                };
                for waiter in start_waiters {
                    let _ = waiter.send(Ok(()));
                }
            }
            StartWorkerResult::Failed(error) => {
                for invocation in queued {
                    let _ = invocation.accepted.send(Err(error.clone()));
                }
                for (_, waiter) in stop_waiters {
                    let _ = waiter.send(Err(error.clone()));
                }
                for waiter in start_waiters {
                    let _ = waiter.send(Err(error.clone()));
                }
                self.phase = SupervisorPhase::Stopped;
            }
            StartWorkerResult::CleanupFailed { error, reason } => {
                for invocation in queued {
                    let _ = invocation.accepted.send(Err(error.clone()));
                }
                for (_, waiter) in stop_waiters {
                    let _ = waiter.send(Err(error.clone()));
                }
                for waiter in start_waiters {
                    let _ = waiter.send(Err(error.clone()));
                }
                self.phase = SupervisorPhase::CleanupBlocked {
                    generation,
                    reason,
                    error,
                };
            }
            StartWorkerResult::Cancelled(cleanup) => {
                let cancelled = PluginError::Cancelled {
                    plugin_id: self.plugin_id.clone(),
                    request_id: "start".to_owned(),
                };
                for invocation in queued {
                    let _ = invocation.accepted.send(Err(cancelled.clone()));
                }
                for (_, waiter) in stop_waiters {
                    let _ = waiter.send(cleanup.clone());
                }
                for waiter in start_waiters {
                    let _ = waiter.send(Err(cancelled.clone()));
                }
                self.phase = match cleanup {
                    Ok(()) => SupervisorPhase::Stopped,
                    Err(error) => SupervisorPhase::CleanupBlocked {
                        generation,
                        reason: (*cancel.borrow()).or(Some(StopReason::ManualStop)),
                        error,
                    },
                };
            }
        }
    }

    async fn complete_generation(
        &mut self,
        generation: u64,
        exit: GenerationExit,
        descriptor: ValidatedLaunchDescriptor,
    ) {
        if !matches!(
            self.phase,
            SupervisorPhase::Running {
                generation: current,
                ..
            } if current == generation
        ) {
            return;
        }
        let GenerationExit {
            expected,
            exit_code,
            cleanup_result,
            stop_reason,
        } = exit;
        if let Err(error) = cleanup_result {
            self.phase = SupervisorPhase::CleanupBlocked {
                generation,
                reason: stop_reason,
                error,
            };
            return;
        }
        let tree_sequence = event_sequence(generation, 2);
        let terminal_sequence = event_sequence(generation, 3);
        let Ok(generation_value) = json_safe(generation) else {
            self.phase = SupervisorPhase::CrashLoop {
                recent_crashes: self.config.crash_threshold.max(1) as u32,
            };
            return;
        };
        let Ok(tree_sequence_value) = json_safe(tree_sequence) else {
            self.phase = SupervisorPhase::CrashLoop {
                recent_crashes: self.config.crash_threshold.max(1) as u32,
            };
            return;
        };
        let Ok(terminal_sequence_value) = json_safe(terminal_sequence) else {
            self.phase = SupervisorPhase::CrashLoop {
                recent_crashes: self.config.crash_threshold.max(1) as u32,
            };
            return;
        };
        if self
            .events
            .record(PluginRuntimeEvent::TreeReaped {
                plugin_id: self.plugin_id.clone(),
                content_owner: descriptor.content_owner.clone(),
                generation: generation_value,
                sequence: tree_sequence_value,
            })
            .await
            .is_err()
        {
            self.phase = SupervisorPhase::CrashLoop {
                recent_crashes: self.config.crash_threshold.max(1) as u32,
            };
            return;
        }
        let event = if expected {
            PluginRuntimeEvent::Stopped {
                plugin_id: self.plugin_id.clone(),
                content_owner: descriptor.content_owner,
                generation: generation_value,
                sequence: terminal_sequence_value,
            }
        } else {
            PluginRuntimeEvent::Crashed {
                plugin_id: self.plugin_id.clone(),
                content_owner: descriptor.content_owner,
                generation: generation_value,
                sequence: terminal_sequence_value,
                exit_code,
            }
        };
        if self.events.record(event).await.is_err() {
            self.phase = SupervisorPhase::CrashLoop {
                recent_crashes: self.config.crash_threshold.max(1) as u32,
            };
            return;
        }

        if expected {
            self.phase = SupervisorPhase::Stopped;
            return;
        }
        let now = Instant::now();
        self.recent_crashes.push_back(now);
        while self
            .recent_crashes
            .front()
            .is_some_and(|instant| now.duration_since(*instant) > self.config.crash_window)
        {
            self.recent_crashes.pop_front();
        }
        if self.recent_crashes.len() >= self.config.crash_threshold {
            self.phase = SupervisorPhase::CrashLoop {
                recent_crashes: self.recent_crashes.len() as u32,
            };
        } else {
            let exponent = self.recent_crashes.len().saturating_sub(1).min(5) as u32;
            self.start_backoff_until = Some(now + std::time::Duration::from_secs(1 << exponent));
            self.phase = SupervisorPhase::Stopped;
        }
    }

    fn state_projection(&self) -> RuntimeState {
        match &self.phase {
            SupervisorPhase::Stopped => RuntimeState::Stopped,
            SupervisorPhase::Starting {
                generation,
                spawn_token,
                cancel,
                cleanup,
                ..
            } => {
                if let Some(reason) = *cleanup.borrow() {
                    RuntimeState::CleanupPending {
                        generation: *generation,
                        process_tree: ProcessTreeToken(*generation),
                        reason,
                    }
                } else if let Some(reason) = *cancel.borrow() {
                    RuntimeState::CancellingStart {
                        generation: *generation,
                        spawn_token: *spawn_token,
                        reason,
                    }
                } else {
                    RuntimeState::Starting {
                        generation: *generation,
                        spawn_token: *spawn_token,
                    }
                }
            }
            SupervisorPhase::Running {
                generation,
                process_id,
                ..
            } => RuntimeState::Running {
                generation: *generation,
                pid: *process_id,
            },
            SupervisorPhase::CrashLoop { recent_crashes } => RuntimeState::CrashLoop {
                recent_crashes: *recent_crashes,
            },
            SupervisorPhase::CleanupBlocked {
                generation,
                reason: Some(reason),
                ..
            } => RuntimeState::CleanupPending {
                generation: *generation,
                process_tree: ProcessTreeToken(*generation),
                reason: *reason,
            },
            SupervisorPhase::CleanupBlocked { .. } => RuntimeState::CrashLoop {
                recent_crashes: self.config.crash_threshold.max(1) as u32,
            },
        }
    }
}

enum StartDemand {
    Invocation(Box<QueuedInvocation>),
    Explicit(oneshot::Sender<Result<(), PluginError>>),
}

impl StartDemand {
    fn fail(self, error: PluginError) {
        match self {
            Self::Invocation(invocation) => {
                let _ = invocation.accepted.send(Err(error));
            }
            Self::Explicit(completed) => {
                let _ = completed.send(Err(error));
            }
        }
    }

    fn into_queues(
        self,
    ) -> (
        Vec<QueuedInvocation>,
        Vec<oneshot::Sender<Result<(), PluginError>>>,
    ) {
        match self {
            Self::Invocation(invocation) => (vec![*invocation], Vec::new()),
            Self::Explicit(completed) => (Vec::new(), vec![completed]),
        }
    }
}
