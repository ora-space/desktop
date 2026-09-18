//! Guardian lifecycle coordination for the new process runtime.

#[cfg(target_os = "linux")]
mod host_state;
#[cfg(target_os = "linux")]
pub use host_state::HostState;
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub use host::{HostCoordinator, serve_process_host};
#[cfg(target_os = "linux")]
mod guardian;
#[cfg(target_os = "linux")]
mod state_error;
#[cfg(target_os = "linux")]
mod state_journal;
#[cfg(target_os = "linux")]
pub use guardian::serve_guardian_bootstrap;
#[cfg(target_os = "linux")]
pub use state_error::ProcessStateError;

#[cfg(target_os = "linux")]
mod linux_best_effort;
#[cfg(target_os = "linux")]
mod linux_helper;
#[cfg(target_os = "linux")]
mod linux_output;
#[cfg(target_os = "linux")]
pub use linux_best_effort::LinuxBestEffort;
mod platform;
mod scope;
mod stop;
#[cfg(target_os = "linux")]
pub use linux_helper::{
    LinuxHelperConfig, check_linux_helper_deployment, serve_linux_helper,
    spawn_linux_helper_workload,
};

pub use platform::{
    ContainmentObservation, OutputPlatform, Platform, PlatformCapabilities, PlatformError,
    PlatformObservation, SpawnError, StopSignal,
};
pub use scope::{AdmissionError, ReconcileFailure, ScopeRuntime, StartError, StopError};
