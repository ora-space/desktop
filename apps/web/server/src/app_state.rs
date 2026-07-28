use crate::service::{FileSystemApi, ProjectWorkContextApi};
use ora_backend::Backend;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Holds the shared state that HTTP handlers need to serve requests.
#[derive(Clone)]
pub struct AppState {
    backend: Backend,
    file_system_api: Arc<FileSystemApi>,
    project_work_context_api: Arc<ProjectWorkContextApi>,
    ready: Arc<AtomicBool>,
}

impl AppState {
    /// Creates one shared application state value with readiness disabled until bootstrap completes.
    pub fn new(
        backend: Backend,
        file_system_api: Arc<FileSystemApi>,
        project_work_context_api: Arc<ProjectWorkContextApi>,
    ) -> Self {
        Self {
            backend,
            file_system_api,
            project_work_context_api,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the shared persisted backend used by the five common CRUD route families.
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Returns the shared read-only filesystem API used by the web path picker.
    pub fn file_system_api(&self) -> &Arc<FileSystemApi> {
        &self.file_system_api
    }

    /// Returns the shared project work context API that routes delegate into.
    pub fn project_work_context_api(&self) -> &Arc<ProjectWorkContextApi> {
        &self.project_work_context_api
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
