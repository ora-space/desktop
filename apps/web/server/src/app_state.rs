use crate::service::{FileSystemApi, WorkspaceFileApi};
use ora_backend::Backend;
use ora_plugin_manager::PluginManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

/// Holds the shared state that HTTP handlers need to serve requests.
#[derive(Clone)]
pub struct AppState {
    backend: Backend,
    file_system_api: Arc<FileSystemApi>,
    workspace_file_api: Arc<WorkspaceFileApi>,
    plugin_manager: Arc<PluginManager>,
    ready: Arc<AtomicBool>,
    shutdown: CancellationToken,
}

impl AppState {
    /// Creates one shared application state value with readiness disabled until bootstrap completes.
    pub fn new(
        backend: Backend,
        file_system_api: Arc<FileSystemApi>,
        workspace_file_api: Arc<WorkspaceFileApi>,
        plugin_manager: Arc<PluginManager>,
    ) -> Self {
        Self {
            backend,
            file_system_api,
            workspace_file_api,
            plugin_manager,
            ready: Arc::new(AtomicBool::new(false)),
            shutdown: CancellationToken::new(),
        }
    }

    /// Returns the process-wide token that HTTP streams observe during graceful shutdown.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Cancels live HTTP streams so Axum can drain connections after Ctrl+C.
    pub fn request_shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Returns the shared persisted backend used by the five common CRUD route families.
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Returns the shared read-only filesystem API used by the web path picker.
    pub fn file_system_api(&self) -> &Arc<FileSystemApi> {
        &self.file_system_api
    }

    /// Returns the shared task-workspace filesystem API used by explorer and viewer routes.
    pub fn workspace_file_api(&self) -> &Arc<WorkspaceFileApi> {
        &self.workspace_file_api
    }
    /// Returns the immutable installed-plugin snapshot captured during bootstrap.
    pub fn plugin_manager(&self) -> &Arc<PluginManager> {
        &self.plugin_manager
    }

    /// Marks the runtime as ready after bootstrap finishes successfully.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Reports whether bootstrap has completed successfully for readiness checks.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}
