//! One session execution from start to its terminal result.
mod conversation;
mod pre_start;
use conversation::{Conversation, TurnBook, pending_cancel_bound, settle_before_start};
use pre_start::{EndDecision, ForgetAcceptances, PreStart, ended_before_start, settling_cancels};

use super::end::SessionEnd;
use super::host::{
    CheckoutDirectory, IgnoredStatus, NodeRuntimeHost, SessionLauncher, SessionLifecycle,
    SessionPlugin, ThreadMirror,
};
use super::ports::{
    CheckoutResolver, CommandSettlement, PluginCatalog, PriorRevisionRestore, SessionCommand,
    SessionLedger,
};
use super::queue::{Plan, plan};
use super::resume;
use super::scheduled;
use super::workload::{PlacementError, SessionPlacement};
use super::{SessionConfig, Shared};
use crate::model_proxy::{ModelAccess, ModelAccessStatus, ModelRoute};
use agent_client_protocol_schema::v1::{ContentBlock as AcpContentBlock, MessageId, TextContent};
use ora_agent_runtime::{AgentRuntimeManager, AgentRuntimeSetup, MemorySessionStore};
use ora_contracts::{
    PromptSessionEvent, PromptSessionRequest, PublicError, StartSessionRequest, StopPluginRequest,
    StopSessionRequest,
};
use ora_domain::{AgentRef, PluginId, SessionId};
use ora_logging::ora_warn;
use ora_node_protocol::{
    AgentSessionSpec, CommandId, ContentBlock, EndSessionReason, ExecutionId, OperationId, UserTurn,
};
use ora_plugin_lifecycle::{GenerationTaps, PluginLifecycle, PluginLifecycleConfig};
use ora_plugin_manager::PluginContribution;
use ora_scheduler::Scheduler;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Notify;

/// The one package the Node may still configure for a model binding without a frozen
/// configuration: the OpenCode build that predates plugin-owned adapters.
const LEGACY_OPENCODE_PLUGIN: &str = "official/ora-space.opencode";

/// Runs one session execution to its end and writes the terminal result.
///
/// Commands still queued when the session ends are settled as discarded before the live entry
/// is removed, and the terminal result is written last, so delivery can never observe an ended
/// session whose history is still being written.
pub(super) async fn run<L, C, P, R>(
    shared: Arc<Shared<L, C, P, R>>,
    operation: OperationId,
    execution: ExecutionId,
    spec: AgentSessionSpec,
    wake: Arc<Notify>,
) -> bool
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    let trigger = spec.trigger;
    let checkout = shared
        .checkouts
        .checkout(&spec.checkout_execution_id)
        .map(|checkout| checkout.path);
    // Whatever acceptance times are left go with the session, on every way out of this run,
    // including cancels the ledger settles while the stop waits for evidence.
    let _acceptance = ForgetAcceptances {
        acceptance: &shared.cancel_acceptance,
        execution: &execution,
    };
    let mut end = prepare_and_converse(&shared, &operation, &execution, spec, &wake).await;
    let ended = end.terminal(shared.node.clone());
    if super::model::seal_unstarted_history(&shared.sessions_root(), &execution).is_err() {
        return false;
    }
    if shared.ledger.defer_restore_end(&execution, &ended).is_err() {
        return false;
    }
    if super::stop::decide(&shared.ledger, &execution, &ended, &shared.config.stop).is_err()
        || super::stop::request(&shared.ledger, &execution, &shared.config.stop).is_err()
    {
        return false;
    }
    shared.stop_spawning(&execution);
    match shared.ledger.session_stop_records(Some(&execution)) {
        Ok(records)
            if records
                .iter()
                .any(|record| record.state == ora_node_db::SessionStopState::StopUnconfirmed) =>
        {
            return true;
        }
        Ok(_) => {}
        Err(_) => return false,
    }
    match super::stop::confirm(
        &shared.ledger,
        &shared.config.home_directory,
        &shared.config.stop,
        &execution,
        true,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return true,
        Err(_) => return false,
    }
    if shared.release_resources(&execution).is_err()
        || super::stop::released(&shared.ledger, &execution).is_err()
    {
        return false;
    }
    if !shared.restorer.confirm_stop(&execution).await {
        // A deliberate handover is not an actor failure and emits no terminal event.
        return true;
    }
    if let Some(checkout) = checkout {
        end.scheduled = scheduled::declare(
            &end,
            trigger,
            &shared.sessions_root(),
            &execution,
            &checkout,
        );
    }
    let ended = end.terminal(shared.node.clone());
    match shared.ledger.queued_commands(&execution) {
        Ok(queued) => {
            // A still-queued cancel is settled as `session_ending` by the terminal write itself,
            // before the terminal event; discarding it would leave it without a settlement.
            for queued in queued
                .into_iter()
                .filter(|queued| !matches!(queued.command, SessionCommand::CancelTurn(_)))
            {
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
/// Nothing is started before the checkout and the exact plugin version are known, and before a
/// prior Revision the session resumes is restored: a session whose plugin is missing, of another
/// version, or whose prior Revision cannot be restored ends without any plugin process having
/// existed.
async fn prepare_and_converse<L, C, P, R>(
    shared: &Arc<Shared<L, C, P, R>>,
    operation: &OperationId,
    execution: &ExecutionId,
    mut spec: AgentSessionSpec,
    wake: &Arc<Notify>,
) -> SessionEnd
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    let Some(cloned) = shared.checkouts.checkout(&spec.checkout_execution_id) else {
        return SessionEnd::agent_failed("checkout_unavailable");
    };
    let checkout = cloned.path.clone();
    // The lease comes first, so the version checked below cannot be replaced before it runs.
    let lease = shared.catalog.lease(&spec.agent_plugin_id);
    let Some(package_root) = shared
        .catalog
        .installed(&spec.agent_plugin_id, &spec.agent_plugin_version)
    else {
        return SessionEnd::agent_failed("agent_plugin_unavailable");
    };
    let Ok(plugin_id) = PluginId::parse(spec.agent_plugin_id.as_str()) else {
        return SessionEnd::agent_failed("agent_plugin_unavailable");
    };
    // Admission checked a declaration Cloud holds; under the lease the Node proves the installed
    // bytes, declaration and Settings are that same one before any model authority exists.
    if let Some(configuration) = &spec.agent_configuration
        && let Err(error) = shared.catalog.check_configuration(configuration)
    {
        return SessionEnd::agent_failed(error.code());
    }
    let mut packages = vec![ora_node_db::SessionStopPackage {
        plugin_id: spec.agent_plugin_id.clone(),
        plugin_version: spec.agent_plugin_version.clone(),
        view_key: None,
    }];
    packages.extend(
        spec.mcp
            .iter()
            .map(|binding| ora_node_db::SessionStopPackage {
                plugin_id: binding.plugin_id.clone(),
                plugin_version: binding.plugin_version.clone(),
                view_key: None,
            }),
    );
    let control = shared
        .controls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(execution)
        .cloned();
    let (session_setup, mut mcp_leases) = match super::mcp::NodeSessionMcp::prepare(
        &shared.catalog,
        if control.is_some() {
            Vec::new()
        } else {
            std::mem::take(&mut spec.mcp)
        },
        &checkout,
    ) {
        Ok(prepared) => prepared,
        Err(failure) => return failure.preflight_end(execution),
    };
    // Taken out of the spec so the values live only in the session view the runtime asks on
    // every setup; a repeated start never reaches here, so the first values stay in use.
    let session_setup = session_setup.with_host_servers(std::mem::take(&mut spec.host_mcp_servers));
    let host_refused = session_setup.host_refused();
    // The checkout still has the clone's owner here; the hand-back below covers what Git wrote.
    let turns = TurnBook::default();
    let initial = spec.initial_turn.turn_id.clone();
    let stopping = shared.stopping.subscribe();
    let restored = resume::restore_prior(
        &shared.restorer,
        &shared.ledger,
        operation,
        execution,
        &cloned,
        &mut spec,
        resume::EndSignals {
            stopping,
            ended: ended_before_start(
                &shared.ledger,
                execution,
                wake,
                &initial,
                &turns,
                &shared.cancel_acceptance,
                EndDecision {
                    node: &shared.node,
                    stop: &shared.config.stop,
                },
            ),
        },
    );
    if let Err(end) = restored.await {
        return end;
    }
    if spec.prior_revision.is_some() && shared.ledger.restored(execution).is_err() {
        return SessionEnd::agent_failed("prior_revision_unavailable");
    }
    // Declared before the lifecycle so that, on every path, it is dropped (and its directory
    // removed) only after the lifecycle and the plugin it ran are gone.
    let placement = {
        let workload = shared.config.workload.clone();
        let execution_id = execution.clone();
        let execution = execution.clone();
        let package_root = package_root.clone();
        let prepared = settling_cancels(
            tokio::task::spawn_blocking(move || {
                SessionPlacement::prepare(&workload, &execution, &package_root, &checkout)
                    .map(|placement| (placement, checkout))
            }),
            PreStart {
                ledger: &shared.ledger,
                execution: &execution_id,
                wake,
                initial: &initial,
                turns: &turns,
                acceptance: &shared.cancel_acceptance,
            },
        )
        .await;
        match prepared {
            Ok(joined) => joined.unwrap_or_else(|error| Err(std::io::Error::other(error).into())),
            Err(end) => return end,
        }
    };
    let (mut placement, checkout) = match placement {
        Ok(prepared) => prepared,
        Err(PlacementError::CheckoutTooDeep) => {
            ora_warn!(execution_id = %execution, "checkout is too deep to hand to the workload user");
            return SessionEnd::agent_failed("checkout_too_deep");
        }
        Err(PlacementError::Io(error)) => {
            ora_warn!(execution_id = %execution, error = %error, "session workload could not be prepared");
            return SessionEnd::agent_failed("agent_start_failed");
        }
    };
    let session_setup = match session_setup.in_placement(&placement) {
        Ok(setup) => setup,
        Err(failure) => return failure.preflight_end(execution),
    };
    let mut model_access = match &spec.model_binding_id {
        Some(binding) => {
            let Some(config) = shared.config.model_proxy.clone() else {
                return SessionEnd::agent_failed("model_proxy_unavailable");
            };
            // A frozen configuration hands the grant to the plugin's adapter; without one only
            // the legacy OpenCode package, whose configuration the Node still writes, may run.
            // Any other plugin would receive a live token it never declared, so it is refused
            // before a grant exists.
            let route = match &spec.agent_configuration {
                Some(configuration) => ModelRoute::Adapter(configuration),
                None if spec.agent_plugin_id.as_str() == LEGACY_OPENCODE_PLUGIN => {
                    ModelRoute::LegacyOpenCode
                }
                None => return SessionEnd::agent_failed("agent_model_configuration_required"),
            };
            let home = &shared.config.home_directory;
            let pre_start = PreStart {
                ledger: &shared.ledger,
                execution,
                wake,
                initial: &initial,
                turns: &turns,
                acceptance: &shared.cancel_acceptance,
            };
            let opened = match settling_cancels(
                ModelAccess::open(config, binding, execution, home, route),
                pre_start,
            )
            .await
            {
                Ok(opened) => opened,
                Err(end) => return end,
            };
            match opened {
                Ok(access) => Some(access),
                Err(error) if error.0 == "model_session_ending" => {
                    return match super::model::reconcile(
                        &shared.ledger,
                        execution,
                        wake,
                        shared.stopping.subscribe(),
                        super::model::END_RECONCILE_WAIT,
                        &mut || {
                            // Nothing is admitted yet: every cancel settles without a decision.
                            settle_before_start(
                                &shared.ledger,
                                execution,
                                &initial,
                                &turns,
                                &shared.cancel_acceptance,
                            )
                            .map(|()| None)
                            .map_err(|_| "ledger_unavailable")
                        },
                    )
                    .await
                    {
                        Ok(reason) => match super::model::seal_unstarted_history(
                            &shared.sessions_root(),
                            execution,
                        ) {
                            Ok(()) => SessionEnd::requested(reason),
                            Err(detail) => SessionEnd::agent_failed(detail),
                        },
                        Err(detail) => SessionEnd::agent_failed(detail),
                    };
                }
                Err(error) => return SessionEnd::agent_failed(error.0),
            }
        }
        None => None,
    };
    if let Some(access) = &mut model_access
        && let Err(error) = placement.publish_model_ca(access)
    {
        access.close().await;
        return SessionEnd::agent_failed(error.0);
    }
    if shared.ledger.agent_admitted(execution).is_err() {
        if let Some(access) = &mut model_access {
            access.close().await;
        }
        return SessionEnd::agent_failed("agent_start_failed");
    }
    let mut environment = model_access
        .as_ref()
        .map(|access| access.environment.clone())
        .unwrap_or_default();
    // The socket exists only while the session runs; dropping it removes the file, and requests
    // that arrive after that fail as `no_session`.
    let ora_git = match &shared.ora_git {
        Some(broker) => match broker.open(
            crate::ora_git::SessionGit {
                execution: execution.clone(),
                checkout_execution: spec.checkout_execution_id.clone(),
                checkout: cloned.clone(),
                directory: spec.workspace_directory.clone(),
            },
            &mut environment,
        ) {
            Ok(socket) => socket,
            Err(error) => {
                ora_warn!(execution_id = %execution, error = %error, "ora-git socket could not be opened");
                if let Some(access) = &mut model_access {
                    access.close().await;
                }
                return SessionEnd::agent_failed("ora_git_unavailable");
            }
        },
        None => None,
    };
    let taps = GenerationTaps::default();
    let isolation =
        super::process::ModelProcessIsolation::for_session(spec.agent_configuration.is_some());
    for package in &mut packages {
        package.view_key = placement.view_key();
    }
    let identity = ora_node_db::SessionStopIdentity {
        attempt: super::stop::attempt_id(),
        execution: execution.clone(),
        checkout: spec.checkout_execution_id.clone(),
        node: shared.node.clone(),
        host: shared.config.stop.host(),
        kind: ora_node_db::SessionStopKind::Agent,
        packages,
    };
    let spawner = match super::stop::prepare(
        &shared.ledger,
        &shared.config.home_directory,
        identity,
        isolation.spawner(placement.identity()),
    ) {
        Ok(spawner) => spawner,
        Err(_) => {
            if let Some(access) = &mut model_access {
                access.close().await;
            }
            return SessionEnd::agent_failed("agent_start_failed");
        }
    };
    let launcher = placement.launcher(
        &spec.git_identity,
        &package_root,
        environment,
        isolation,
        spawner.clone(),
    );
    let agent_home = model_access.as_ref().map_or_else(
        || checkout.clone(),
        |access| {
            placement.home_directory().map_or_else(
                || std::path::PathBuf::from(&access.environment["HOME"]),
                std::path::Path::to_path_buf,
            )
        },
    );
    placement.retain_for_stop();
    mcp_leases.push(lease);
    shared
        .retained
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            execution.clone(),
            super::stop::Retained {
                placement,
                leases: mcp_leases,
                spawner: spawner.clone(),
                isolation,
            },
        );
    let lifecycle = match open_lifecycle(&shared.config, launcher, &taps) {
        Ok(lifecycle) => lifecycle,
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "plugin lifecycle could not open");
            if let Some(access) = &mut model_access {
                access.close().await;
            }
            return SessionEnd::agent_failed("agent_plugin_unavailable");
        }
    };
    if !runs_package(&lifecycle, &plugin_id, &package_root) {
        if let Some(access) = &mut model_access {
            access.close().await;
        }
        return SessionEnd::agent_failed("agent_plugin_unavailable");
    }

    let mirror = ThreadMirror::new(
        shared.ledger.clone(),
        operation.clone(),
        execution.clone(),
        wake.clone(),
        shared.previews.clone(),
    );
    let session_setup = if let Some(control) = &control {
        let shared = Arc::downgrade(shared);
        let execution = execution.clone();
        let checkout = checkout.clone();
        let cache = std::sync::Mutex::new(
            None::<(ora_node_db::AttemptId, u64, super::mcp::NodeSessionMcp)>,
        );
        session_setup.with_lifecycle(control.clone(), move |input| {
            let mut cached = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let shared = shared
                .upgrade()
                .ok_or_else(|| super::mcp::NodeMcpFailure::plugin("session".into()))?;
            let attempt = shared
                .retained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&execution)
                .ok_or_else(|| super::mcp::NodeMcpFailure::plugin("session".into()))?
                .spawner
                .identity()
                .attempt;
            if let Some((cached_attempt, generation, setup)) = &*cached
                && *generation == input.generation
                && cached_attempt == &attempt
            {
                return Ok(setup.clone());
            }
            let (setup, leases) =
                super::mcp::NodeSessionMcp::prepare_update(&shared.catalog, input, &checkout)?;
            let mut retained = shared
                .retained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retained = retained
                .get_mut(&execution)
                .ok_or_else(|| super::mcp::NodeMcpFailure::plugin("session".into()))?;
            let setup = setup.in_placement(&retained.placement)?;
            retained.leases.extend(leases);
            *cached = Some((attempt, input.generation, setup.clone()));
            Ok(setup)
        })
    } else {
        session_setup
    };
    let session_setup = if control.is_some() {
        super::mcp::configure(
            session_setup.with_initial_packages(&spec.mcp),
            shared,
            execution,
            lifecycle.clone(),
            plugin_id.clone(),
            checkout.clone(),
        )
    } else {
        session_setup
    };
    let scheduler = Scheduler::new(shared.config.timezone);
    let manager = AgentRuntimeManager::<NodeRuntimeHost<L>>::new(AgentRuntimeSetup {
        attach: Arc::new(SessionPlugin {
            process_admission: control
                .as_ref()
                .map(|_| super::host::SessionProcessAdmission {
                    setup: session_setup.clone(),
                    last: tokio::sync::Mutex::new(None),
                }),
            spawner: spawner.clone(),
            lifecycle: lifecycle.clone(),
            taps,
            plugin_id: plugin_id.clone(),
            model_context: model_access
                .as_ref()
                .and_then(|access| access.context.clone()),
        }),
        store: MemorySessionStore::default(),
        directory: CheckoutDirectory(checkout.clone()),
        session_setup,
        skill_catalog: ora_agent_runtime::NoSkillCatalog,
        events: mirror.clone(),
        home_directory: agent_home,
        sessions_root: shared.sessions_root(),
        scheduler: scheduler.clone(),
    });
    let end = match manager {
        Ok(manager) => {
            if let Some(control) = &control {
                super::mcp::probes::register(
                    shared,
                    execution,
                    &spec.checkout_execution_id,
                    checkout.clone(),
                    control.clone(),
                );
            }
            let conversation = Conversation {
                ledger: &shared.ledger,
                execution,
                session_id: SessionId::new(execution.as_str()),
                manager: &manager,
                mirror: &mirror,
                wake,
                control: control.as_ref(),
                started: std::sync::atomic::AtomicBool::new(false),
                blocked: std::sync::Mutex::new(std::collections::HashMap::new()),
                node: shared.node.clone(),
                keep: conversation::Keep {
                    on_cancel: spec.keep_session_on_cancel,
                    on_failure: spec.keep_session_on_failure,
                },
                boundary_verified: shared
                    .config
                    .turn_cancel
                    .verifies(&spec.agent_plugin_id, &spec.agent_plugin_version),
                processes: super::stop::turn::TurnProcesses::new(spawner),
                turn_cancel: &shared.config.turn_cancel,
                stop_config: &shared.config.stop,
                cancel_acceptance: &shared.cancel_acceptance,
                turns: &turns,
            };
            let mut stopping = shared.stopping.subscribe();
            let mut model_status = model_access.as_ref().map(|access| access.status.clone());
            let agent_ref = AgentRef::for_plugin(&plugin_id);
            let end = tokio::select! {
                biased;
                _ = async { if let Some(control) = &control { loop { control.probes.notified().await; shared.probes.dispatch(); } } else { std::future::pending::<()>().await; } } => unreachable!(),
                _ = async { if let Some(control) = &control { loop { control.refresh.notified().await; manager.notify_mcp_desired_changed(); } } else { std::future::pending::<()>().await; } } => unreachable!(),
                _ = async { let _ = stopping.wait_for(|stop| *stop).await; } => conversation.stop(SessionEnd::requested(EndSessionReason::Cancelled)).await,
                // A refresh or replacement onto an Agent without HTTP MCP fails off the turn path;
                // the execution ends instead of continuing without its host servers.
                () = host_refused.notified() => {
                    ora_warn!(execution_id = %execution, error_code = super::mcp::host::HOST_MCP_HTTP_CAPABILITY_MISSING, "host MCP servers need an Agent with HTTP MCP");
                    conversation.stop(SessionEnd::agent_failed(super::mcp::host::HOST_MCP_HTTP_CAPABILITY_MISSING)).await
                },
                status = async {
                    match &mut model_status {
                        Some(status) => status.wait_for(|status| *status != ModelAccessStatus::Active).await
                            .map_or(ModelAccessStatus::Revoked, |status| *status),
                        None => std::future::pending::<ModelAccessStatus>().await,
                    }
                } => {
                    match status {
                        // The real reason arrives with the Controller's `EndSession`; recording
                        // the provisional `model_access_revoked` first would let a restart publish
                        // it, so the decision waits for the reconciled reason.
                        ModelAccessStatus::Ending => {
                            // Stopping the runtime and revoking the grant can each take long, so
                            // they run beside the wait for the reason: the decision, with every
                            // pending cancel's settlement, is recorded by the cancels' deadline
                            // whatever the cleanup is doing (D1, D3).
                            let cleanup = async {
                                conversation.stop_undecided().await;
                                if let Some(access) = &mut model_access {
                                    access.close().await;
                                }
                            };
                            let decided = async {
                                let end = match super::model::reconcile(&shared.ledger, execution, wake,
                                    shared.stopping.subscribe(), super::model::END_RECONCILE_WAIT,
                                    &mut || pending_cancel_bound(&shared.ledger, execution,
                                        &shared.cancel_acceptance, shared.config.turn_cancel.grace)).await {
                                    Ok(reason) => SessionEnd::requested(reason),
                                    Err(detail) => SessionEnd::agent_failed(detail),
                                };
                                match super::stop::decide(&shared.ledger, execution,
                                    &end.terminal(shared.node.clone()), &shared.config.stop) {
                                    Ok(()) => end,
                                    Err(_) => SessionEnd::agent_failed("ledger_unavailable"),
                                }
                            };
                            let ((), end) = tokio::join!(cleanup, decided);
                            end
                        }
                        ModelAccessStatus::Active | ModelAccessStatus::Revoked => {
                            conversation.stop(SessionEnd::agent_failed("model_access_revoked")).await
                        }
                    }
                },
                end = conversation.run(&agent_ref, spec, shared.config.agent_ready_timeout) => end,
            };
            shared.probes.unregister(execution);
            // The session leaves the runtime here: its ora-git socket goes before the plugin
            // does, once a request it was still serving has ended.
            if let Some(socket) = ora_git {
                socket.close().await;
            }
            // Dropping the manager releases its connection supervisor, which stops reconnecting.
            drop(manager);
            end
        }
        Err(error) => {
            ora_warn!(execution_id = %execution, error = %error, "agent runtime could not start");
            drop(ora_git);
            SessionEnd::agent_failed("agent_start_failed")
        }
    };
    // The first deadline and terminal intention precede process cleanup; proof is checked by run.
    if super::stop::decide(
        &shared.ledger,
        execution,
        &end.terminal(shared.node.clone()),
        &shared.config.stop,
    )
    .is_err()
        || super::stop::request(&shared.ledger, execution, &shared.config.stop).is_err()
    {
        return SessionEnd::agent_failed("ledger_unavailable");
    }
    shared.stop_spawning(execution);
    if let Err(error) = super::stop::await_plugin_stop(
        &shared.ledger,
        &shared.config.home_directory,
        &shared.config.stop,
        execution,
        lifecycle.stop_plugin(StopPluginRequest {
            plugin_id: plugin_id.to_string(),
        }),
    )
    .await
    {
        ora_warn!(execution_id = %execution, error = %error, "agent stop proof unavailable");
    }
    scheduler.shutdown().await;
    if let Some(access) = &mut model_access {
        access.close().await;
    }
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
