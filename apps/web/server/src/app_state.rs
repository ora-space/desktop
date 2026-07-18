use crate::service::{
    AgentApi, FileSystemApi, ProjectApi, ProjectWorkContextApi, SessionApi, SkillApi, TaskApi,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Holds the shared state that HTTP handlers need to serve requests.
#[derive(Clone)]
pub struct AppState {
    agent_api: Arc<AgentApi>,
    file_system_api: Arc<FileSystemApi>,
    project_api: Arc<ProjectApi>,
    project_work_context_api: Arc<ProjectWorkContextApi>,
    task_api: Arc<TaskApi>,
    session_api: Arc<SessionApi>,
    skill_api: Arc<SkillApi>,
    ready: Arc<AtomicBool>,
}

impl AppState {
    /// Creates one shared application state value with readiness disabled until bootstrap completes.
    pub fn new(
        agent_api: Arc<AgentApi>,
        file_system_api: Arc<FileSystemApi>,
        project_api: Arc<ProjectApi>,
        project_work_context_api: Arc<ProjectWorkContextApi>,
        task_api: Arc<TaskApi>,
        session_api: Arc<SessionApi>,
        skill_api: Arc<SkillApi>,
    ) -> Self {
        Self {
            agent_api,
            file_system_api,
            project_api,
            project_work_context_api,
            task_api,
            session_api,
            skill_api,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the shared configurable-agent API used by HTTP routes.
    pub fn agent_api(&self) -> &Arc<AgentApi> {
        &self.agent_api
    }

    /// Returns the shared read-only filesystem API used by the web path picker.
    pub fn file_system_api(&self) -> &Arc<FileSystemApi> {
        &self.file_system_api
    }

    /// Returns the shared project API that routes delegate into.
    pub fn project_api(&self) -> &Arc<ProjectApi> {
        &self.project_api
    }

    /// Returns the shared project work context API that routes delegate into.
    pub fn project_work_context_api(&self) -> &Arc<ProjectWorkContextApi> {
        &self.project_work_context_api
    }

    /// Returns the shared task API that routes delegate into.
    pub fn task_api(&self) -> &Arc<TaskApi> {
        &self.task_api
    }

    /// Returns the shared session API that routes delegate into.
    pub fn session_api(&self) -> &Arc<SessionApi> {
        &self.session_api
    }

    /// Returns the shared skill API used by HTTP routes.
    pub fn skill_api(&self) -> &Arc<SkillApi> {
        &self.skill_api
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
