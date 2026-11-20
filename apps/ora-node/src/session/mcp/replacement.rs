//! Exclusive Node process replacement preserves the conversation and its provider identity.
use super::*;
use crate::session::{
    CheckoutResolver, PluginCatalog, PriorRevisionRestore, SessionLedger, Shared,
    host::SessionLifecycle, stop,
};
use ora_node_db::{SessionStopChange, SessionStopPackage};
use std::sync::PoisonError;

/// The callback captures ownership handles, never a second copy of the desired settings.
pub(in crate::session) fn configure<L, C, P, R>(
    setup: NodeSessionMcp,
    shared: &Arc<Shared<L, C, P, R>>,
    execution: &ExecutionId,
    lifecycle: SessionLifecycle,
    plugin: PluginId,
    cwd: std::path::PathBuf,
) -> NodeSessionMcp
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    let weak = Arc::downgrade(shared);
    let execution = execution.clone();
    setup.with_replacement(move |retire| {
        let shared = weak.upgrade();
        let execution = execution.clone();
        let lifecycle = lifecycle.clone();
        let plugin = plugin.clone();
        let cwd = cwd.clone();
        Box::pin(async move {
            let Some(shared) = shared else {
                retire.await;
                return Err(blocked());
            };
            replace(&shared, &execution, &lifecycle, &plugin, &cwd, retire).await
        })
    })
}

/// The old spawn gate closes before cleanup; no fresh gate exists without affirmative proof.
async fn replace<L, C, P, R>(
    shared: &Arc<Shared<L, C, P, R>>,
    execution: &ExecutionId,
    lifecycle: &SessionLifecycle,
    plugin: &PluginId,
    cwd: &Path,
    retire: Retire<'_>,
) -> Result<SessionMcpRevision, RuntimeError>
where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    let prepare_stop = || {
        let spawner = shared
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
            .ok_or_else(blocked)?
            .spawner
            .clone();
        let old = spawner.identity();
        let control = shared
            .controls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
            .cloned()
            .ok_or_else(blocked)?;
        let now = stop::now_ms();
        let bound = shared
            .ledger
            .session_stop_records(Some(execution))
            .map_err(|_| blocked())?
            .iter()
            .find(|record| record.identity.attempt == old.attempt)
            .and_then(|record| record.bound_at_ms)
            .unwrap_or_else(|| {
                now.saturating_add(shared.config.stop.confirmation_timeout_ms as i64)
            });
        shared
            .ledger
            .change_session_stop(
                &old.attempt,
                SessionStopChange::Request {
                    at_ms: now,
                    bound_at_ms: bound,
                },
            )
            .map_err(|_| blocked())?;
        Ok((spawner, old, control, bound))
    };
    let (spawner, old, control, bound) = match prepare_stop() {
        Ok(prepared) => prepared,
        Err(error) => {
            // A suspended supervisor still owns cleanup if recording its stop request fails.
            retire.await;
            return Err(error);
        }
    };
    let _ = spawner
        .stop_for_replacement(&shared.config.stop.host(), async {
            retire.await;
            lifecycle
                .stop_plugin(ora_contracts::StopPluginRequest {
                    plugin_id: plugin.to_string(),
                })
                .await
        })
        .await;
    let scope = stop::recorded_scope(&shared.ledger, &old).ok_or_else(blocked)?;
    let mut pause = stop::FIRST_RECHECK;
    loop {
        if control.snapshot(false).convergence == ora_node_protocol::SessionMcpConvergence::Stopping
        {
            return Err(blocked());
        }
        let files =
            stop::files::StopFiles::open(&shared.config.home_directory).map_err(|_| blocked())?;
        let (evidence, proof) = files
            .settle(
                &old,
                scope,
                &shared.config.stop.host(),
                shared.config.stop.single_mount_with_storage_fencing,
            )
            .await;
        if evidence == crate::managed::attempt_lock::StopEvidence::Stopped {
            shared
                .ledger
                .change_session_stop(
                    &old.attempt,
                    SessionStopChange::Confirm {
                        at_ms: stop::now_ms(),
                        proof: proof.ok_or_else(blocked)?,
                    },
                )
                .map_err(|_| blocked())?;
            break;
        }
        if stop::now_ms() >= bound {
            shared
                .ledger
                .change_session_stop(&old.attempt, SessionStopChange::Unconfirmed)
                .map_err(|_| blocked())?;
            return Err(blocked());
        }
        tokio::time::sleep(pause).await;
        pause = stop::next_recheck(pause);
    }
    let control = shared
        .controls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(execution)
        .cloned()
        .ok_or_else(blocked)?;
    let input = control.desired();
    let (setup, mut leases) = NodeSessionMcp::prepare_update(&shared.catalog, &input, cwd)
        .map_err(NodeMcpFailure::into_runtime_error)?;
    let revision = setup
        .revision()
        .map_err(NodeMcpFailure::into_runtime_error)?;
    let mut resources = shared
        .retained
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let retained = resources.get_mut(execution).ok_or_else(blocked)?;
    setup
        .in_placement(&retained.placement)
        .map_err(NodeMcpFailure::into_runtime_error)?;
    let agent = old.packages.first().cloned().ok_or_else(blocked)?;
    leases.push(shared.catalog.lease(&agent.plugin_id));
    let mut packages = vec![agent];
    packages.extend(input.members.iter().filter_map(|member| match member {
        ora_node_protocol::SessionMcpUpdateMember::Configured {
            plugin_id,
            plugin_version,
            ..
        } => Some(SessionStopPackage {
            plugin_id: plugin_id.clone(),
            plugin_version: plugin_version.clone(),
            view_key: retained.placement.view_key(),
        }),
        _ => None,
    }));
    let next = ora_node_db::SessionStopIdentity {
        attempt: stop::attempt_id(),
        packages,
        ..old.clone()
    };
    let inner = retained.isolation.spawner(retained.placement.identity());
    let next = stop::prepare(&shared.ledger, &shared.config.home_directory, next, inner)
        .map_err(|_| blocked())?;
    retained.leases = leases;
    drop(resources);
    spawner.replace(next);
    super::resources::release(shared, &old).map_err(|_| blocked())?;
    Ok(revision)
}

/// This typed failure blocks a turn without inventing a provider session or ending the execution.
fn blocked() -> RuntimeError {
    RuntimeError::internal(
        "session process replacement pending",
        control::Admission::Blocked,
    )
}
