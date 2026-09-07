mod agent;
mod agent_runtime;
mod app_event;
mod bootstrap;
mod clock;
mod effect_registration;
mod effect_worker;
mod effects;
mod error;
mod git_cleanup;
mod identity;
mod marketplace_sources;
mod plugin;
mod plugin_configuration;
mod plugin_gateway;
mod project;
mod proxy;
mod repository_work;
mod request_lifecycle;
mod session;
mod session_history;
mod session_setup;
mod settings;
mod skill;
mod skill_reconciliation;
mod source_boundary;
mod task;
#[cfg(test)]
mod test_backend;
#[cfg(test)]
mod test_clock;
mod workflow;
mod workspace;

pub use agent::AgentApi;
pub use agent_runtime::{AgentRuntime, SessionEventStream};
pub use app_event::AppEventHub;
pub use bootstrap::{Backend, BackendBootstrapError, BackendPaths};
pub use effects::Effects;
pub use error::{BackendError, ErrorClassification};
pub use identity::resolve_git_identity;
pub use plugin::Plugins;
pub use plugin_gateway::{GatewayError, PluginGateway};
pub use project::ProjectApi;
pub use request_lifecycle::{RequestIdGenerator, RequestLifecycle, UuidRequestIdGenerator};
pub use session::Sessions;
pub use settings::{BackendPreferredLogLevelStore, Settings};
pub use skill::SkillApi;
pub use skill_reconciliation::SkillStorageReconciliationError;
pub use task::TaskApi;
pub use workflow::WorkflowApi;
pub use workflow::run::WorkflowRuns;
pub use workspace::WorkspaceApi;

#[cfg(test)]
mod local_agent_package_tests;
