use std::process::ExitCode;

/// Keeps argument parsing outside the runtime's privileged deployment checks.
fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(target_os = "linux")]
    {
        match run(&arguments) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ora-process-helper: {error}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = arguments;
        eprintln!("ora-process-helper deployment is supported only on Linux");
        ExitCode::FAILURE
    }
}

/// Keeps service startup explicit and leaves installation and account provisioning to deployment.
#[cfg(target_os = "linux")]
fn run(arguments: &[std::ffi::OsString]) -> Result<(), String> {
    use std::path::Path;

    match arguments {
        [operation, config] if operation == "--check" => {
            ora_process_runtime::check_linux_helper_deployment(Path::new(config))?;
            println!("deployment prerequisites checked; workload launch is not available");
            Ok(())
        }
        [operation, config, socket] if operation == "--serve" => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(async {
                use tokio::signal::unix::{SignalKind, signal};
                let mut terminate = signal(SignalKind::terminate()).map_err(|error| error.to_string())?;
                let mut interrupt = signal(SignalKind::interrupt()).map_err(|error| error.to_string())?;
                ora_process_runtime::serve_linux_helper(Path::new(config), Path::new(socket), async {
                    tokio::select! {
                        _ = terminate.recv() => {},
                        _ = interrupt.recv() => {},
                    }
                }).await
            })
        }
        _ => Err("usage: ora-process-helper --check <absolute-config-path> | --serve <absolute-config-path> <absolute-socket-path>".into()),
    }
}
