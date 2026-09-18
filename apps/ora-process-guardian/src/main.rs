use std::process::ExitCode;

/// Accepts only the fixed bootstrap mode; identities and secrets are never command-line arguments.
fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    if std::env::args_os().skip(1).collect::<Vec<_>>() == ["--bootstrap"] {
        return match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => {
                eprintln!("ora-process-guardian: bootstrap or service failed");
                ExitCode::FAILURE
            }
        };
    }
    eprintln!("ora-process-guardian requires an inherited Linux bootstrap channel and scope lock");
    ExitCode::FAILURE
}

/// Converts the explicitly mapped descriptors before initializing the single-threaded service.
#[cfg(target_os = "linux")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use tokio::signal::unix::{SignalKind, signal};
    let lock = std::fs::File::from(take_bootstrap_descriptor(/*fd*/ 0)?);
    // stdout is deliberately a bidirectional Unix socket, never a diagnostic output stream.
    let bootstrap = std::os::unix::net::UnixStream::from(take_bootstrap_descriptor(/*fd*/ 1)?);
    bootstrap.set_nonblocking(/*nonblocking*/ true)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        ora_process_runtime::serve_guardian_bootstrap(
            lock,
            tokio::net::UnixStream::from_std(bootstrap)?,
            async {
                tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
            },
        )
        .await
    })?;
    Ok(())
}

/// Validates a fixed inherited slot and restores CLOEXEC before closing that bootstrap-only slot.
#[cfg(target_os = "linux")]
fn take_bootstrap_descriptor(fd: std::os::fd::RawFd) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // SAFETY: fcntl validates fd; only a successful new descriptor becomes an OwnedFd.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the bootstrap slots are owned exclusively by this dedicated entry point.
    unsafe {
        libc::close(fd);
    }
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(duplicate) })
}
