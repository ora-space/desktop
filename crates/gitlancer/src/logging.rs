use std::path::Path;
use std::sync::{Arc, OnceLock};

/// Observability hook for git command execution in gitlancer.
///
/// Implement this trait to receive events before and after each spawned git
/// process. The registry holds at most one logger for the process lifetime;
/// when none is registered all call sites are no-ops.
pub trait GitlancerLogger: Send + Sync + 'static {
    /// Called immediately before the git process is spawned.
    ///
    /// `cwd` is `GitCommand::cwd` — the directory the git process runs in,
    /// not the host program's working directory.
    /// `command` is the full command string, e.g. `"git status --porcelain=v2"`.
    fn log_command(&self, cwd: &Path, command: &str);

    /// Called after the git process exits or fails to spawn.
    ///
    /// `duration_ms` is zero when the process failed to start.
    /// `exit_code` is `None` when the process was killed by a signal or failed to start.
    fn log_result(&self, duration_ms: u64, success: bool, exit_code: Option<i32>);
}

static LOGGER: OnceLock<Arc<dyn GitlancerLogger>> = OnceLock::new();

/// Registers a process-wide logger for gitlancer git command execution.
///
/// Only the first call takes effect; subsequent calls are silently ignored so
/// callers do not need to coordinate registration order.
pub fn register(logger: impl GitlancerLogger + 'static) {
    let _ = LOGGER.set(Arc::new(logger));
}

/// Returns the registered logger if one has been set.
pub(crate) fn get() -> Option<Arc<dyn GitlancerLogger>> {
    LOGGER.get().cloned()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{GitlancerLogger, register};

    #[test]
    fn register_never_panics_on_repeated_calls() {
        struct NoOpLogger;
        impl GitlancerLogger for NoOpLogger {
            fn log_command(&self, _: &Path, _: &str) {}
            fn log_result(&self, _: u64, _: bool, _: Option<i32>) {}
        }
        register(NoOpLogger);
        register(NoOpLogger);
    }
}
