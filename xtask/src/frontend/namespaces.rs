//! Namespace-scoped endpoint declarations for the generated frontend SDK.

mod agent;
mod agent_import;
mod agent_runtime;
mod app_events;
mod developer_mode;
mod file_system;
mod git;
mod plugin;
mod project;
mod proxy;
mod runtime_log_level;
mod session;
mod skill;
mod skill_import;
mod spec;
mod task;
mod workflow;
mod workflow_run;
mod workspace;

use super::FrontendEndpoint;

/// Builds one ordered catalog by flattening the namespace-owned endpoint slices.
///
/// Keeping the slices separate makes additions visible in the namespace that owns the generated
/// client surface; the exporter only needs a temporary flat view while rendering TypeScript.
pub(super) fn frontend_endpoints() -> Vec<FrontendEndpoint> {
    [
        project::ENDPOINTS,
        developer_mode::ENDPOINTS,
        runtime_log_level::ENDPOINTS,
        task::ENDPOINTS,
        session::ENDPOINTS,
        agent_runtime::ENDPOINTS,
        app_events::ENDPOINTS,
        skill::ENDPOINTS,
        skill_import::ENDPOINTS,
        agent::ENDPOINTS,
        agent_import::ENDPOINTS,
        plugin::ENDPOINTS,
        proxy::ENDPOINTS,
        file_system::ENDPOINTS,
        git::ENDPOINTS,
        spec::ENDPOINTS,
        workflow::ENDPOINTS,
        workflow_run::ENDPOINTS,
        workspace::ENDPOINTS,
    ]
    .into_iter()
    .flat_map(|endpoints| endpoints.iter().copied())
    .collect()
}
