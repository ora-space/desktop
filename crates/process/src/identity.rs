//! The OS identity a spawned process tree runs as.

/// Which identity [`crate::TokioProcessSpawner`] gives the processes it starts.
///
/// Desktop and unprivileged hosts run children as themselves. A privileged host that executes
/// untrusted code (an agent and the tools it starts) names a non-root identity instead, so that
/// code can neither read the host's private state nor own what the host owns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcessIdentity {
    /// The child keeps the spawning process's credentials.
    #[default]
    Inherit,
    /// The child irreversibly drops to this identity before it executes anything, with no
    /// supplementary groups, no capabilities, `no_new_privs`, and an owner-private umask.
    #[cfg(target_os = "linux")]
    Linux(ora_utils::process::LinuxChildIdentity),
}
