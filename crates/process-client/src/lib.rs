//! Guardian discovery, host binding and Run transport; no runtime, database or spawner dependency.

#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub use host::ProcessHost;

#[cfg(target_os = "linux")]
mod runs;
#[cfg(target_os = "linux")]
pub use runs::GuardianRuns;

#[cfg(target_os = "linux")]
mod guardian;
#[cfg(target_os = "linux")]
pub use guardian::GuardianProbe;
#[cfg(target_os = "linux")]
mod management;
#[cfg(target_os = "linux")]
pub use management::GuardianManagement;
#[cfg(target_os = "linux")]
mod transport;
