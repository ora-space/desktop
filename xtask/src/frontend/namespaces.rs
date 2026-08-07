//! Namespace-scoped endpoint declarations for the generated frontend SDK.

mod agent;
mod agent_import;
mod agent_runtime;
mod app_events;
mod file_system;
mod git;
mod plugin;
mod project;
mod project_work_context;
mod session;
mod skill;
mod skill_import;
mod spec;
mod task;
mod workflow;
mod workflow_run;

use super::FrontendEndpoint;

/// Builds one ordered catalog by flattening the namespace-owned endpoint slices.
///
/// Keeping the slices separate makes additions visible in the namespace that owns the generated
/// client surface; the exporter only needs a temporary flat view while rendering TypeScript.
pub(super) fn frontend_endpoints() -> Vec<FrontendEndpoint> {
    [
        project::ENDPOINTS,
        project_work_context::ENDPOINTS,
        task::ENDPOINTS,
        session::ENDPOINTS,
        agent_runtime::ENDPOINTS,
        app_events::ENDPOINTS,
        skill::ENDPOINTS,
        skill_import::ENDPOINTS,
        agent::ENDPOINTS,
        agent_import::ENDPOINTS,
        plugin::ENDPOINTS,
        file_system::ENDPOINTS,
        git::ENDPOINTS,
        spec::ENDPOINTS,
        workflow::ENDPOINTS,
        workflow_run::ENDPOINTS,
    ]
    .into_iter()
    .flat_map(|endpoints| endpoints.iter().copied())
    .collect()
}
