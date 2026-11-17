//! A detached owner keeps process evidence and leases until cleanup is positively confirmed.
use super::*;
use crate::session::{
    CheckoutResolver, PluginCatalog, PriorRevisionRestore, SessionLedger, Shared, stop,
};
use ora_node_db::{SessionStopChange, SessionStopIdentity, SessionStopKind, SessionStopPackage};
use ora_node_protocol::{SessionMcpHealth, SessionMcpUnknownReason};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio};
use ora_utils::mcp::ProbeError;
use std::{path::PathBuf, time::Duration};

/// Resolution runs after permit assignment and uses exactly the session's package placement.
pub(crate) fn register<L, C, P, R>(
    shared: &Arc<Shared<L, C, P, R>>,
    execution: &ExecutionId,
    checkout: &ExecutionId,
    cwd: PathBuf,
    control: Control,
) where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    let weak = Arc::downgrade(shared);
    let id = execution.clone();
    let checkout = checkout.clone();
    shared
        .probes
        .register(execution.clone(), control.clone(), move |input| {
            let shared = weak.upgrade();
            let execution = id.clone();
            let checkout = checkout.clone();
            let cwd = cwd.clone();
            let control = control.clone();
            Box::pin(async move {
                if let Some(shared) = shared {
                    let health = control.clone();
                    let flight = Arc::clone(&input);
                    let result = tokio::spawn(run(
                        Arc::clone(&shared),
                        execution,
                        checkout,
                        cwd,
                        control,
                        input,
                    ))
                    .await;
                    if result.is_err() {
                        // A lost owner is fatal to this Node, never an untracked free slot. Recovery
                        // reconstructs the durable attempt before admitting new probe work.
                        health.probe_result(
                            &flight,
                            SessionMcpHealth::Unknown {
                                reason: SessionMcpUnknownReason::ContextMissing,
                            },
                            true,
                        );
                        shared
                            .failed
                            .store(true, std::sync::atomic::Ordering::Release);
                        std::future::pending::<()>().await;
                    }
                }
            })
        });
}

/// The handshake has eight seconds; the remaining two seconds belong exclusively to cleanup.
async fn run<L, C, P, R>(
    shared: Arc<Shared<L, C, P, R>>,
    execution: ExecutionId,
    checkout: ExecutionId,
    cwd: PathBuf,
    control: Control,
    input: Arc<ProbeInput>,
) where
    L: SessionLedger,
    C: CheckoutResolver,
    P: PluginCatalog,
    R: PriorRevisionRestore,
{
    let admitted = tokio::time::Instant::now();
    let prepared = (|| {
        let selected = super::super::control::Input {
            generation: input.generation,
            members: vec![input.member.clone()],
        };
        let (setup, leases) =
            super::super::NodeSessionMcp::prepare_update(&shared.catalog, &selected, &cwd).ok()?;
        let retained = shared
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let placement = &retained.get(&execution)?.placement;
        let setup = setup.in_placement(placement).ok()?;
        let snapshot = setup
            .snapshot(
                &cwd,
                ora_agent_runtime::AgentSessionMcpCapabilities::new(true, true),
            )
            .ok()?;
        let server = snapshot.into_servers().into_iter().next()?;
        let SessionMcpUpdateMember::Configured {
            plugin_id,
            plugin_version,
            ..
        } = &input.member
        else {
            return None;
        };
        let identity = SessionStopIdentity {
            attempt: stop::attempt_id(),
            execution: execution.clone(),
            checkout,
            node: shared.node.clone(),
            host: shared.config.stop.host(),
            kind: if matches!(
                server,
                agent_client_protocol_schema::v1::McpServer::Stdio(_)
            ) {
                SessionStopKind::StdioProbe
            } else {
                SessionStopKind::HttpProbe
            },
            packages: vec![SessionStopPackage {
                plugin_id: plugin_id.clone(),
                plugin_version: plugin_version.clone(),
                view_key: placement.view_key(),
            }],
        };
        let spawner = stop::prepare(
            &shared.ledger,
            &shared.config.home_directory,
            identity.clone(),
            crate::session::process::ModelProcessIsolation::Isolated.spawner(placement.identity()),
        )
        .ok()?;
        Some((server, leases, identity, spawner))
    })();
    let Some((server, leases, identity, spawner)) = prepared else {
        control.probe_result(
            &input,
            SessionMcpHealth::Unknown {
                reason: SessionMcpUnknownReason::ContextMissing,
            },
            false,
        );
        return;
    };
    let mut leases = Some(leases);
    let result = tokio::time::timeout_at(admitted + Duration::from_secs(8), async {
        use agent_client_protocol_schema::v1::McpServer;
        match server {
            McpServer::Stdio(server) => {
                let mut spec = ProcessSpec::new(server.command)
                    .args(server.args)
                    .cwd(cwd)
                    .stderr(ProcessStdio::Null);
                let mut environment = std::collections::BTreeMap::new();
                crate::session::process::ModelProcessIsolation::Isolated
                    .extend_environment(&mut environment);
                for variable in server.env {
                    environment.insert(variable.name, variable.value);
                }
                for (name, value) in environment {
                    spec = spec.env(name, value);
                }
                let mut child = spawner.spawn(spec).map_err(|_| ProbeError::SpawnFailed)?;
                let mut stdin = child.take_stdin().ok_or(ProbeError::SpawnFailed)?;
                let stdout = child.take_stdout().ok_or(ProbeError::SpawnFailed)?;
                let result = ora_utils::mcp::probe_stdio_handshake(
                    &mut stdin,
                    &mut tokio::io::BufReader::new(stdout),
                )
                .await;
                drop(stdin);
                let _ = child.kill().await;
                let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
                result
            }
            McpServer::Http(server) => {
                ora_utils::mcp::probe_http(
                    server
                        .url
                        .parse()
                        .map_err(|_| ProbeError::HttpUnreachable)?,
                    server
                        .headers
                        .into_iter()
                        .map(|header| (header.name, header.value))
                        .collect(),
                    Duration::from_secs(8),
                )
                .await
            }
            _ => Err(ProbeError::HandshakeFailed),
        }
    })
    .await
    .unwrap_or(Err(ProbeError::Timeout));
    spawner.close();
    let now = stop::now_ms();
    let bound = now.saturating_add(shared.config.stop.confirmation_timeout_ms as i64);
    if shared
        .ledger
        .change_session_stop(
            &identity.attempt,
            SessionStopChange::Request {
                at_ms: now,
                bound_at_ms: bound,
            },
        )
        .is_err()
    {
        // A journal outage cannot release the slot or its exact package leases.
        std::future::pending::<()>().await;
    }
    let mut health = result.map_or_else(
        |error| SessionMcpHealth::Unhealthy { code: code(error) },
        |()| SessionMcpHealth::Healthy,
    );
    let mut timed_out = false;
    let mut unconfirmed = false;
    let mut confirmed = false;
    loop {
        if !timed_out && tokio::time::Instant::now() >= admitted + Duration::from_secs(10) {
            timed_out = true;
            health = SessionMcpHealth::Unhealthy {
                code: ora_node_protocol::SessionMcpProbeErrorCode::McpProbeTimeout,
            };
            control.probe_result(&input, health.clone(), true);
        }
        // After the D5 bound there is no automatic polling or signal loop. Root reconciliation
        // wakes owners; only its durable confirmation can release the retained capacity.
        let notified = shared.probes.1.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if unconfirmed {
            confirmed = shared
                .ledger
                .session_stop_records(Some(&execution))
                .ok()
                .is_some_and(|records| {
                    records.iter().any(|record| {
                        record.identity.attempt == identity.attempt
                            && record.state == ora_node_db::SessionStopState::Confirmed
                    })
                });
            if !confirmed {
                if timed_out {
                    notified.await;
                } else {
                    tokio::select! { () = notified => {}, () = tokio::time::sleep_until(admitted + Duration::from_secs(10)) => {} }
                }
                continue;
            }
        }
        let proof = stop::files::StopFiles::open(&shared.config.home_directory)
            .ok()
            .and_then(|files| {
                files.terminate(&identity, &shared.config.stop.host());
                let (evidence, proof) = files.probe(
                    &identity,
                    &shared.config.stop.host(),
                    shared.config.stop.single_mount_with_storage_fencing,
                );
                (evidence == crate::managed::attempt_lock::StopEvidence::Stopped)
                    .then_some(proof)
                    .flatten()
            });
        if !confirmed
            && let Some(proof) = proof
            && shared
                .ledger
                .change_session_stop(
                    &identity.attempt,
                    SessionStopChange::Confirm {
                        at_ms: stop::now_ms(),
                        proof,
                    },
                )
                .is_ok()
        {
            confirmed = true;
        }
        if confirmed {
            leases.take();
            if super::super::resources::release(shared.as_ref(), &identity).is_ok() {
                control.probe_result(&input, health, false);
                return;
            }
        }
        if !unconfirmed && !confirmed && stop::now_ms() >= bound {
            unconfirmed = shared
                .ledger
                .change_session_stop(&identity.attempt, SessionStopChange::Unconfirmed)
                .is_ok();
        }
        tokio::time::sleep(if timed_out {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(100)
        })
        .await;
    }
}

/// Product errors never contain server output, request headers or resolved settings.
fn code(error: ProbeError) -> ora_node_protocol::SessionMcpProbeErrorCode {
    use ora_node_protocol::SessionMcpProbeErrorCode as Code;
    match error {
        ProbeError::SpawnFailed => Code::McpSpawnFailed,
        ProbeError::ExitedPrematurely => Code::McpExitedPrematurely,
        ProbeError::HandshakeFailed => Code::McpHandshakeFailed,
        ProbeError::Timeout => Code::McpProbeTimeout,
        ProbeError::ToolsUnavailable => Code::McpToolsUnavailable,
        ProbeError::HttpUnreachable => Code::McpHttpUnreachable,
        ProbeError::HttpUnauthorized => Code::McpHttpUnauthorized,
        ProbeError::HttpServerError => Code::McpHttpServerError,
    }
}
