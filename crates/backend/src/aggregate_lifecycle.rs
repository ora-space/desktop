use crate::{BackendError, BackendErrorKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

/// Coordinates task provisioning with Project deletion without serializing unrelated projects.
#[derive(Debug, Default)]
pub(crate) struct ProjectTaskLifecycleCoordinator {
    projects: Mutex<HashMap<String, Arc<RwLock<()>>>>,
}

impl ProjectTaskLifecycleCoordinator {
    /// Returns the stable asynchronous lock shared by operations on one Project.
    pub(crate) fn project(&self, project_id: &str) -> Result<Arc<RwLock<()>>, BackendError> {
        let mut projects = self.projects.lock().map_err(|_| coordination_error())?;
        Ok(projects
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone())
    }
}

/// Builds the stable error used when lifecycle coordination has been poisoned.
fn coordination_error() -> BackendError {
    BackendError::new(
        BackendErrorKind::Internal,
        "aggregate_lifecycle_error",
        "aggregate lifecycle coordination is unavailable",
    )
}
