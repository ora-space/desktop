use crate::config::DesktopConfigStore;
use ora_backend::Backend;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Holds the shared Backend and Desktop configuration store managed by Tauri.
#[derive(Clone)]
pub struct DesktopState {
    pub backend: Backend,
    pub config: DesktopConfigStore,
    /// The Tauri application data directory, owner of the dashboard locator files.
    pub app_data_directory: PathBuf,
    pub stream_cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

/// Retains process-scoped writer guards for the full Tauri application lifetime.
pub struct DesktopRuntimeGuard {
    pub _logging: ora_logging::LoggingGuard,
}
