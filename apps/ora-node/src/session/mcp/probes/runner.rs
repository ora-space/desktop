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
                    run(shared, execution, checkout, cwd, control, input).await;
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
                for variable in server.env {
                    spec = spec.env(variable.name, variable.value);
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
    if shared
        .ledger
        .change_session_stop(
            &identity.attempt,
            SessionStopChange::Request {
                at_ms: now,
                bound_at_ms: now.saturating_add(2000),
            },
        )
        .is_err()
    {
        // A journal outage cannot release the slot or its exact package leases.
        std::future::pending::<()>().await;
    }
    let health = result.map_or_else(
        |error| SessionMcpHealth::Unhealthy { code: code(error) },
        |()| SessionMcpHealth::Healthy,
    );
    loop {
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
        if let Some(proof) = proof
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
            leases.take();
            if super::super::resources::release(shared.as_ref(), &identity).is_ok() {
                control.probe_result(&input, health, false);
                return;
            }
        }
        if tokio::time::Instant::now() >= admitted + Duration::from_secs(10) {
            let _ = shared
                .ledger
                .change_session_stop(&identity.attempt, SessionStopChange::Unconfirmed);
            control.probe_result(
                &input,
                SessionMcpHealth::Unknown {
                    reason: SessionMcpUnknownReason::ContextMissing,
                },
                true,
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
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
