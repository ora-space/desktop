use std::process::ExitCode;

/// Runs a standalone Node with optional explicitly configured local Controller IPC.
fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args
            .first()
            .is_some_and(|arg| arg == "reconcile-session-stop")
        {
            return match reconcile(&args[1..]) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("ora-node: reconciliation failed: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        if args.len() == 1 {
            return match run(std::path::Path::new(&args[0])) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("ora-node: startup or recovery failed: {error}");
                    ExitCode::FAILURE
                }
            };
        }
    }
    eprintln!("usage (Linux): ora-node <absolute-config-file>");
    ExitCode::FAILURE
}

/// Uses the running Node's root-only socket; even an offline invocation never opens its database.
#[cfg(target_os = "linux")]
fn reconcile(args: &[std::ffi::OsString]) -> Result<(), Box<dyn std::error::Error>> {
    use ora_node_db::{AttemptId, SessionStopMode, SessionStopReason, SessionStopReconcile};
    let [home, execution, attempt, request, reason, mode] = args else {
        return Err("usage: ora-node reconcile-session-stop <absolute-home> <execution> <attempt> <request-id> <maintenance|probe-capacity|legacy-upgrade> <recheck|after-host-reboot>".into());
    };
    let text = |value: &std::ffi::OsString| {
        value
            .to_str()
            .map(str::to_owned)
            .ok_or("arguments must be UTF-8")
    };
    let request = SessionStopReconcile {
        request_id: text(request)?,
        execution: ora_node_protocol::ExecutionId::new(text(execution)?),
        attempt: AttemptId::parse(&text(attempt)?).ok_or("invalid attempt ID")?,
        reason: match text(reason)?.as_str() {
            "maintenance" => SessionStopReason::Maintenance,
            "probe-capacity" => SessionStopReason::ProbeCapacity,
            "legacy-upgrade" => SessionStopReason::LegacyUpgrade,
            _ => return Err("invalid reason".into()),
        },
        mode: match text(mode)?.as_str() {
            "recheck" => SessionStopMode::Recheck,
            "after-host-reboot" => SessionStopMode::AfterHostReboot,
            _ => return Err("invalid mode".into()),
        },
    };
    let reply = ora_node::reconcile_session_stop(std::path::Path::new(home), &request)?;
    println!("{}", serde_json::to_string(&reply)?);
    if matches!(reply, ora_node::SessionStopAdminReply::Rejected { .. }) {
        return Err("request rejected".into());
    }
    Ok(())
}

/// Installs shutdown handling before recovery or IPC starts; no Backend entry point is installed.
#[cfg(target_os = "linux")]
fn run(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::signal::unix::{SignalKind, signal};
    if !path.is_absolute() {
        return Err("configuration path must be absolute".into());
    }
    let config: ora_node::ServiceConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    if config
        .control
        .as_ref()
        .is_some_and(|c| matches!(c.listen, ora_node::ControlListen::WebSocket { .. }))
    {
        return Err(
            "network management requires mutual_tls_websocket; plaintext is unsupported".into(),
        );
    }
    if config
        .control
        .as_ref()
        .is_some_and(|c| matches!(c.listen, ora_node::ControlListen::MutualTlsWebSocket { .. }))
        && config.process.workload_uid.is_none()
    {
        return Err("network management requires a separate workload OS identity".into());
    }
    let _logging = ora_logging::init_logging(ora_logging::LoggingConfig::new(
        ora_logging::LogLevel::Info,
        ora_logging::LogOutput::Stdout,
        config.timezone.parse()?,
    ))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    serve_then_exit(runtime, async move {
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let shutdown = ora_node::Shutdown::default();
        // The reaper ends the Git this Node runs if the Node is killed. A Node that cannot
        // start it does not start, and one that loses it stops instead of running on.
        ora_process::initialize_reaper(config.process.reaper()?)
            .map_err(|error| format!("process reaper could not be started: {error}"))?;
        let service = ora_node::serve(config, shutdown.clone());
        tokio::pin!(service);
        let served = tokio::select! {
            result = &mut service => result,
            _ = terminate.recv() => { shutdown.request(); service.await },
            _ = interrupt.recv() => { shutdown.request(); service.await },
        };
        // Every Git has been settled by now. Ending the reaper here means a Node that exited
        // on its own leaves no process behind for its supervisor to wait for; a reaper that
        // already died has nothing left to answer and is not the failure to report.
        let _ = ora_process::shutdown_reaper();
        served?;
        match shutdown.failure() {
            Some(reason) => Err(reason.into()),
            None => Ok::<_, Box<dyn std::error::Error>>(()),
        }
    })
}

/// Runs the Node to completion on `runtime`, then exits the runtime within [`RUNTIME_SHUTDOWN`]
/// even if a blocking task never returns.
///
/// By now the service and its worker have returned, and the bounded shutdown abandons every
/// blocking task still running, whatever it is:
/// - a stop observation (terminate, probe, or both) whose budget ran out, still blocked in an
///   `environ` read on a stuck mount. This is the only kind expected here. It only signals and
///   reads procfs; the caller records the outcome in the ledger, and that caller gave up, so an
///   abandoned observation leaves no ledger row behind, only the Unknown its caller recorded;
/// - the service worker, ora-git requests, checkout placement, a revision restore, or a file
///   read of a revision upload. The service awaits each of these before it returns, so none is
///   expected to remain. One left on a failure path ends as it would if the Node were killed:
///   its ledger writes are SQLite transactions, and restart recovery handles the step it was in.
///
/// Dropping the runtime instead would wait for such a task indefinitely; exiting ends its thread
/// with the process.
#[cfg(target_os = "linux")]
fn serve_then_exit<T>(
    runtime: tokio::runtime::Runtime,
    node: impl std::future::Future<Output = Result<T, Box<dyn std::error::Error>>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let served = runtime.block_on(node);
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
    served
}

/// How long an exiting Node waits for blocking tasks it no longer awaits.
#[cfg(target_os = "linux")]
const RUNTIME_SHUTDOWN: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{RUNTIME_SHUTDOWN, serve_then_exit};
    use std::time::{Duration, Instant};

    /// A blocking task nothing awaits any more, as a stop observation hung in an `environ` read
    /// is, holds the exit for the bound and no longer.
    #[test]
    fn a_hung_blocking_task_holds_the_exit_only_for_the_bound()
    -> Result<(), Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (release, hung) = std::sync::mpsc::channel::<()>();
        let began = Instant::now();
        serve_then_exit(runtime, async move {
            let (started, running) = tokio::sync::oneshot::channel();
            // Dropping the handle abandons the task, as an observation's caller does at its
            // budget. It must be running by then: a task still queued is simply discarded. It
            // gives up on its own well past the bound, so an exit that waited for it fails the
            // assertion instead of hanging the test.
            drop(tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                let _ = hung.recv_timeout(RUNTIME_SHUTDOWN * 3);
            }));
            running.await?;
            Ok(())
        })?;
        let waited = began.elapsed();
        drop(release);
        assert!(
            waited >= RUNTIME_SHUTDOWN && waited < RUNTIME_SHUTDOWN + Duration::from_secs(2),
            "{waited:?}"
        );
        Ok(())
    }
}
