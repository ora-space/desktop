use std::process::ExitCode;

/// Runs a standalone Node with optional explicitly configured local Controller IPC.
fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
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
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async move {
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
        })?;
    Ok(())
}
