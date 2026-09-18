use std::process::ExitCode;

/// Opens a standalone recovery owner; no Controller transport or Backend entry point is installed.
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
    eprintln!("usage (Linux): ora-node <absolute-config-file>; no Controller IPC is provided");
    ExitCode::FAILURE
}

/// Installs shutdown handling before starting recovery; all state paths come from the configuration.
#[cfg(target_os = "linux")]
fn run(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use ora_node::{Node, NodeConfig, ProcessConfig, Shutdown};
    use serde::Deserialize;
    use std::time::{Duration, Instant};
    use tokio::signal::unix::{SignalKind, signal};

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Config {
        node: NodeConfig,
        process: ProcessConfig,
        timezone: String,
        recovery_interval_ms: u64,
        #[serde(default)]
        clone: Option<ora_node::CloneConfig>,
    }
    if !path.is_absolute() {
        return Err("configuration path must be absolute".into());
    }
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    if config.recovery_interval_ms == 0 {
        return Err("recovery interval must be positive".into());
    }
    let _logging = ora_logging::init_logging(ora_logging::LoggingConfig::new(
        ora_logging::LogLevel::Info,
        ora_logging::LogOutput::Stdout,
        config.timezone.parse()?,
    ))?;
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async move {
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let shutdown = Shutdown::default();
        let worker_shutdown = shutdown.clone();
        let mut worker = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let mut node = Node::open(config.node, config.process, worker_shutdown.clone()).map_err(|e| e.to_string())?;
            if let Some(clone) = config.clone { node.configure_clone(clone).map_err(|e| e.to_string())?; }
            ora_logging::ora_info!(node_id = %node.node_id().as_str(), "Node opened without Controller IPC");
            let mut previous = None;
            while !worker_shutdown.requested() {
                node.recover().map_err(|e| e.to_string())?;
                let state = node.recover_clones().map_err(|e| e.to_string())?;
                if previous != Some(state) {
                    ora_logging::ora_info!(state = ?state, "Node recovery pass completed");
                    previous = Some(state);
                }
                let next = Instant::now() + Duration::from_millis(config.recovery_interval_ms);
                while !worker_shutdown.requested() && Instant::now() < next {
                    std::thread::sleep(Duration::from_millis(/*millis*/ 25));
                }
            }
            node.shutdown().map_err(|e| e.to_string())
        });
        let result = tokio::select! {
            result = &mut worker => result,
            _ = terminate.recv() => { shutdown.request(); worker.await },
            _ = interrupt.recv() => { shutdown.request(); worker.await },
        };
        result?.map_err(std::io::Error::other)?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
