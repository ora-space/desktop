//! One session execution from start to its terminal result.

use super::host::{
    CheckoutDirectory, IgnoredStatus, NodeRuntimeHost, SessionLauncher, SessionLifecycle,
    SessionPlugin, ThreadMirror,
};
use super::ports::{CheckoutResolver, CommandSettlement, PluginCatalog, SessionLedger};
use super::queue::{Plan, plan};
use super::workload::SessionPlacement;
use super::{SessionConfig, Shared};
use agent_client_protocol_schema::v1::{ContentBlock as AcpContentBlock, MessageId, TextContent};
use ora_agent_runtime::{AgentRuntimeManager, AgentRuntimeSetup, MemorySessionStore, NoSessionMcp};
use ora_contracts::{
    PromptSessionEvent, PromptSessionRequest, StartSessionRequest, StopPluginRequest,
    StopSessionRequest,
};
use ora_domain::{AgentRef, PluginId, SessionId};
use ora_logging::ora_warn;
use ora_node_protocol::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionSpec, CommandId, ContentBlock,
    EndSessionReason, ExecutionId, UserTurn,
};
use ora_plugin_lifecycle::{GenerationTaps, PluginLifecycle, PluginLifecycleConfig};
use ora_plugin_manager::PluginContribution;
use ora_scheduler::Scheduler;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Notify;

/// Why a session execution ended, before the Node identity is attached.
struct SessionEnd {
    reason: AgentSessionEndReason,
    /// A bounded code naming the cause of an `agent_failed` end.
    detail: Option<&'static str>,
}

impl SessionEnd {
    /// Ends the session because the agent could not be run or kept running.
    fn agent_failed(detail: &'static str) -> Self {
        Self {
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some(detail),
        }
    }

    /// Ends the session as the command asked.
    fn requested(reason: EndSessionReason) -> Self {
        Self {
            reason: match reason {
                EndSessionReason::UserEnded => AgentSessionEndReason::UserEnded,
                EndSessionReason::IdleTimeout => AgentSessionEndReason::IdleTimeout,
                EndSessionReason::Cancelled => AgentSessionEndReason::Cancelled,
            },
            detail: None,
        }
    }
}

/// Runs one session execution to its end and writes the terminal result.
///
/// Commands still queued when the session ends are settled as discarded before the live entry
/// is removed, and the terminal result is written last, so delivery can never observe an ended
/// session whose history is still being written.
pub(super) async fn run<L, C, P>(
    shared: Arc<Shared<L, C, P>>,
    execution: ExecutionId,
    spec: AgentSessionSpec,
    wake: Arc<Notify>,
) -> bool
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
{
    let end = prepare_and_converse(&shared, &execution, spec, &wake).await;
    match shared.ledger.queued_commands(&execution) {
        Ok(queued) => {
            for queued in queued {
                settle(
                    &shared.ledger,
                    &execution,
                    &queued.command_id,
                    CommandSettlement::Discarded,
                );
            }
        }
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "session queue unreadable at end");
        }
    }
    let ended = AgentSessionEnded {
        node: shared.node.clone(),
        reason: end.reason,
        detail: end.detail.map(str::to_string),
    };
    match shared.ledger.end_session(&execution, ended) {
        Ok(_) => true,
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "session terminal result was not written");
            false
        }
    }
}

/// Checks what the session needs, composes its runtime, converses, and tears everything down.
///
/// Nothing is started before the checkout and the exact plugin version are known: a session whose
/// plugin is missing or of another version ends without any plugin process having existed.
async fn prepare_and_converse<L, C, P>(
    shared: &Shared<L, C, P>,
    execution: &ExecutionId,
    spec: AgentSessionSpec,
    wake: &Arc<Notify>,
) -> SessionEnd
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
{
    let Some(checkout) = shared.checkouts.checkout(&spec.checkout_execution_id) else {
        return SessionEnd::agent_failed("checkout_unavailable");
    };
    // The lease comes first, so the version checked below cannot be replaced before it runs.
    let _lease = shared.catalog.lease(&spec.agent_plugin_id);
    let Some(package_root) = shared
        .catalog
        .installed(&spec.agent_plugin_id, &spec.agent_plugin_version)
    else {
        return SessionEnd::agent_failed("agent_plugin_unavailable");
    };
    let Ok(plugin_id) = PluginId::parse(spec.agent_plugin_id.as_str()) else {
        return SessionEnd::agent_failed("agent_plugin_unavailable");
    };
    // Declared before the lifecycle so that, on every path, it is dropped (and its directory
    // removed) only after the lifecycle and the plugin it ran are gone.
    let placement = {
        let workload = shared.config.workload.clone();
        let execution = execution.clone();
        let package_root = package_root.clone();
        tokio::task::spawn_blocking(move || {
            SessionPlacement::prepare(&workload, &execution, &package_root, &checkout)
                .map(|placement| (placement, checkout))
        })
        .await
        .unwrap_or_else(|error| Err(std::io::Error::other(error)))
    };
    let (placement, checkout) = match placement {
        Ok(prepared) => prepared,
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "session workload could not be prepared");
            return SessionEnd::agent_failed("agent_start_failed");
        }
    };
    let taps = GenerationTaps::default();
    let launcher = placement.launcher(&spec.git_identity, &package_root);
    let lifecycle = match open_lifecycle(&shared.config, launcher, &taps) {
        Ok(lifecycle) => lifecycle,
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "plugin lifecycle could not open");
            return SessionEnd::agent_failed("agent_plugin_unavailable");
        }
    };
    if !runs_package(&lifecycle, &plugin_id, &package_root) {
        return SessionEnd::agent_failed("agent_plugin_unavailable");
    }

    let mirror = ThreadMirror::new(shared.ledger.clone(), execution.clone(), wake.clone());
    let scheduler = Scheduler::new(shared.config.timezone);
    let manager = AgentRuntimeManager::<NodeRuntimeHost<L>>::new(AgentRuntimeSetup {
        attach: Arc::new(SessionPlugin {
            lifecycle: lifecycle.clone(),
            taps,
            plugin_id: plugin_id.clone(),
        }),
        store: MemorySessionStore::default(),
        directory: CheckoutDirectory(checkout.clone()),
        session_setup: NoSessionMcp::default(),
        events: mirror.clone(),
        home_directory: checkout,
        sessions_root: shared.sessions_root(),
        scheduler: scheduler.clone(),
    });
    let end = match manager {
        Ok(manager) => {
            let conversation = Conversation {
                ledger: &shared.ledger,
                execution,
                session_id: SessionId::new(execution.as_str()),
                manager: &manager,
                mirror: &mirror,
                wake,
            };
            let mut stopping = shared.stopping.subscribe();
            let agent_ref = AgentRef::for_plugin(&plugin_id);
            let end = tokio::select! {
                biased;
                _ = async { let _ = stopping.wait_for(|stop| *stop).await; } => conversation.stop(SessionEnd::requested(EndSessionReason::Cancelled)).await,
                end = conversation.run(&agent_ref, spec, shared.config.agent_ready_timeout) => end,
            };
            // Dropping the manager releases its connection supervisor, which stops reconnecting.
            drop(manager);
            end
        }
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "agent runtime could not start");
            SessionEnd::agent_failed("agent_start_failed")
        }
    };
    // The plugin's whole process tree is gone once this returns, before the lease is released.
    if let Err(error) = lifecycle
        .stop_plugin(StopPluginRequest {
            plugin_id: plugin_id.to_string(),
        })
        .await
    {
        ora_warn!(execution_id = %execution, error = %error, "agent plugin did not stop cleanly");
    }
    scheduler.shutdown().await;
    // The plugin is gone, so nothing reads the package view or writes the session home anymore.
    drop(placement);
    end
}

/// Opens a lifecycle over the Node's plugin root that launches with this session's placement.
fn open_lifecycle(
    config: &SessionConfig,
    launcher: SessionLauncher,
    taps: &GenerationTaps,
) -> Result<SessionLifecycle, ora_plugin_lifecycle::PluginLifecycleError> {
    PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: config.home_directory.clone(),
            deno_path: config.deno_path.clone(),
        },
        launcher,
        IgnoredStatus,
        taps.clone(),
    )
}

/// Confirms the lifecycle would launch an agent from exactly the catalog's version directory.
fn runs_package(lifecycle: &SessionLifecycle, plugin_id: &PluginId, package_root: &Path) -> bool {
    let Some(plugin) = lifecycle.installed_plugin(plugin_id) else {
        return false;
    };
    let same_root = match (
        plugin.package_root.canonicalize(),
        package_root.canonicalize(),
    ) {
        (Ok(discovered), Ok(expected)) => discovered == expected,
        (Err(_), _) | (_, Err(_)) => false,
    };
    same_root && matches!(plugin.contributes, PluginContribution::Agent(_))
}

/// Settles one command, logging a failure: the ledger settles leftovers with the terminal result.
fn settle<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
    command_id: &CommandId,
    settlement: CommandSettlement,
) -> bool {
    match ledger.settle_command(execution, command_id, settlement) {
        Ok(()) => true,
        Err(error) => {
            ora_warn!(execution_id = %execution, command_id = %command_id, error = %error, "session command was not settled");
            false
        }
    }
}

/// How one running turn stopped.
enum TurnOutcome {
    Finished,
    EndRequested {
        command_id: CommandId,
        reason: EndSessionReason,
        discarded: Vec<CommandId>,
    },
    ThreadBroken,
    LedgerUnavailable,
}

/// The conversation of one session execution over its composed runtime.
struct Conversation<'a, L: SessionLedger> {
    ledger: &'a L,
    execution: &'a ExecutionId,
    session_id: SessionId,
    manager: &'a AgentRuntimeManager<NodeRuntimeHost<L>>,
    mirror: &'a ThreadMirror<L>,
    wake: &'a Notify,
}

impl<L: SessionLedger> Conversation<'_, L> {
    /// Opens the session, runs the initial turn, then serves queued commands until one ends it.
    async fn run(
        &self,
        agent_ref: &AgentRef,
        spec: AgentSessionSpec,
        ready_timeout: std::time::Duration,
    ) -> SessionEnd {
        match tokio::time::timeout(ready_timeout, self.manager.wait_for_agent(agent_ref)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                ora_warn!(execution_id = %self.execution, error = %error, "agent did not become ready");
                return SessionEnd::agent_failed("agent_unavailable");
            }
            Err(_) => return SessionEnd::agent_failed("agent_unavailable"),
        }
        if let Err(error) = self
            .manager
            .start_session_with_id(
                self.session_id.clone(),
                StartSessionRequest {
                    workspace_id: spec.checkout_execution_id.to_string(),
                    agent_ref: agent_ref.to_string(),
                    model: None,
                },
            )
            .await
        {
            ora_warn!(execution_id = %self.execution, error = %error, "agent session did not start");
            return SessionEnd::agent_failed("agent_start_failed");
        }

        let mut next_turn = Some(spec.initial_turn);
        loop {
            if self.mirror.is_broken() {
                return self
                    .stop(SessionEnd::agent_failed("thread_unavailable"))
                    .await;
            }
            let turn = match next_turn.take() {
                Some(turn) => turn,
                None => match self.ledger.queued_commands(self.execution).map(plan) {
                    Ok(Plan::Turn { command_id, turn }) => {
                        if !settle(
                            self.ledger,
                            self.execution,
                            &command_id,
                            CommandSettlement::Executed,
                        ) {
                            return self
                                .stop(SessionEnd::agent_failed("ledger_unavailable"))
                                .await;
                        }
                        turn
                    }
                    Ok(Plan::End {
                        command_id,
                        reason,
                        discarded,
                    }) => return self.end_requested(command_id, reason, discarded).await,
                    Ok(Plan::Wait) => {
                        self.wake.notified().await;
                        continue;
                    }
                    Err(error) => {
                        ora_warn!(execution_id = %self.execution, error = %error, "session queue unreadable");
                        return self
                            .stop(SessionEnd::agent_failed("ledger_unavailable"))
                            .await;
                    }
                },
            };
            match self.run_turn(turn).await {
                Ok(TurnOutcome::Finished) => {}
                Ok(TurnOutcome::EndRequested {
                    command_id,
                    reason,
                    discarded,
                }) => return self.end_requested(command_id, reason, discarded).await,
                Ok(TurnOutcome::ThreadBroken) => {
                    return self
                        .stop(SessionEnd::agent_failed("thread_unavailable"))
                        .await;
                }
                Ok(TurnOutcome::LedgerUnavailable) => {
                    return self
                        .stop(SessionEnd::agent_failed("ledger_unavailable"))
                        .await;
                }
                Err(end) => return self.stop(end).await,
            }
        }
    }

    /// Runs one user turn until the agent completes it or an `EndSession` interrupts it.
    ///
    /// Records settled while the turn runs, the cancellation an end causes included, belong to
    /// it; the attribution is cleared only once the turn can produce nothing more.
    async fn run_turn(&self, turn: UserTurn) -> Result<TurnOutcome, SessionEnd> {
        self.mirror.set_turn(Some(turn.turn_id.clone()));
        let prompt = turn
            .content
            .into_iter()
            .map(|block| match block {
                ContentBlock::Text { text } => AcpContentBlock::Text(TextContent::new(text)),
            })
            .collect();
        let stream = self
            .manager
            .prompt_session_as_message(
                PromptSessionRequest {
                    session_id: self.session_id.to_string(),
                    prompt,
                    model: None,
                    record_prompt: None,
                },
                MessageId::new(turn.turn_id.as_str()),
            )
            .await;
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                self.mirror.set_turn(None);
                ora_warn!(execution_id = %self.execution, error = %error, "turn was not admitted");
                return Err(SessionEnd::agent_failed("agent_unavailable"));
            }
        };
        let outcome = loop {
            tokio::select! {
                event = stream.recv() => match event {
                    Some(Ok(PromptSessionEvent::Completed { .. })) | Some(Err(_)) | None => {
                        break TurnOutcome::Finished;
                    }
                    Some(Ok(_)) => {}
                },
                () = self.wake.notified() => {
                    if self.mirror.is_broken() {
                        break TurnOutcome::ThreadBroken;
                    }
                    match self.ledger.queued_commands(self.execution).map(plan) {
                        Ok(Plan::End { command_id, reason, discarded }) => {
                            break TurnOutcome::EndRequested { command_id, reason, discarded };
                        }
                        Ok(Plan::Turn { .. } | Plan::Wait) => {}
                        Err(error) => {
                            ora_warn!(execution_id = %self.execution, error = %error, "session queue unreadable");
                            break TurnOutcome::LedgerUnavailable;
                        }
                    }
                }
            }
        };
        match outcome {
            TurnOutcome::Finished => self.mirror.set_turn(None),
            // The turn is still running: stopping cancels it and records its end under this turn.
            TurnOutcome::EndRequested { .. }
            | TurnOutcome::ThreadBroken
            | TurnOutcome::LedgerUnavailable => {}
        }
        drop(stream);
        Ok(outcome)
    }

    /// Ends the session as a command asked: queued turns ahead of it never run.
    async fn end_requested(
        &self,
        command_id: CommandId,
        reason: EndSessionReason,
        discarded: Vec<CommandId>,
    ) -> SessionEnd {
        for discarded in &discarded {
            settle(
                self.ledger,
                self.execution,
                discarded,
                CommandSettlement::Discarded,
            );
        }
        let end = self.stop(SessionEnd::requested(reason)).await;
        settle(
            self.ledger,
            self.execution,
            &command_id,
            CommandSettlement::Executed,
        );
        end
    }

    /// Stops the session, cancelling a running turn and closing the provider session.
    async fn stop(&self, end: SessionEnd) -> SessionEnd {
        if let Err(error) = self
            .manager
            .stop_session(StopSessionRequest {
                session_id: self.session_id.to_string(),
            })
            .await
        {
            ora_warn!(execution_id = %self.execution, error = %error, "agent session did not stop cleanly");
        }
        self.mirror.set_turn(None);
        end
    }
}
