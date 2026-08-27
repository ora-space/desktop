mod agent;
mod agent_runtime;
mod app_event;
mod bootstrap;
mod clock;
mod effect_surface_registration;
mod effect_worker;
mod error;
mod git_cleanup;
mod identity;
mod marketplace_sources;
mod plugin;
mod plugin_configuration;
mod plugin_gateway;
mod project;
mod proxy;
mod request_lifecycle;
mod session;
mod session_history;
mod skill;
mod skill_reconciliation;
mod source_boundary;
mod spec;
mod task;
mod user_config;
mod workflow;
mod workspace_diff;

pub use agent_runtime::SessionEventStream;
pub use app_event::AppEventHub;
pub use bootstrap::{Backend, BackendBootstrapError, BackendPaths};
pub use error::{BackendError, ErrorClassification};
pub use plugin_gateway::{GatewayError, PluginGateway};
pub use request_lifecycle::{RequestIdGenerator, RequestLifecycle, UuidRequestIdGenerator};
pub use skill_reconciliation::SkillStorageReconciliationError;
pub use user_config::BackendPreferredLogLevelStore;

#[cfg(test)]
mod plugin_install_tests;
