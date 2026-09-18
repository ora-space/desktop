use std::process::ExitCode;

/// Explicit mode and state paths prevent an accidental fresh host after failed recovery.
fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        if args.len() == 3 && (args[0] == "create" || args[0] == "recover") {
            return match run(&args) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => {
                    eprintln!(
                        "ora-process-host: initialization or service failed; existing state preserved"
                    );
                    ExitCode::FAILURE
                }
            };
        }
    }
    eprintln!(
        "usage (Linux): ora-process-host <create|recover> <absolute-state-dir> <absolute-guardian-executable>"
    );
    ExitCode::FAILURE
}

/// Process arguments inject deployment paths; neither HOME nor workload cwd selects host state.
#[cfg(target_os = "linux")]
fn run(args: &[std::ffi::OsString]) -> Result<(), Box<dyn std::error::Error>> {
    use ora_process_runtime::{HostState, serve_process_host};
    use std::path::{Path, PathBuf};
    use tokio::signal::unix::{SignalKind, signal};
    let guardian = PathBuf::from(&args[2]);
    if !guardian.is_absolute() {
        return Err("guardian path must be absolute".into());
    }
    let state = if args[0] == "create" {
        HostState::create(Path::new(&args[1]))?
    } else {
        HostState::recover(Path::new(&args[1]))?
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut terminate = signal(SignalKind::terminate())?;
            let mut interrupt = signal(SignalKind::interrupt())?;
            serve_process_host(state, guardian, async {
                tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
            })
            .await
        })?;
    Ok(())
}
