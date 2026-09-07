//! Desktop-owned build-time bindings consumed by the contract exporter, never by the runtime.
//!
//! Public operation metadata remains transport-neutral. This catalog alone owns concrete Rust
//! handlers and command grants, including native commands outside the contracts SDK.

#[path = "bindings/agent.rs"]
mod agent;
#[path = "bindings/agent_import.rs"]
mod agent_import;
#[path = "bindings/agent_runtime.rs"]
mod agent_runtime;
#[path = "bindings/app_events.rs"]
mod app_events;
#[path = "bindings/developer_mode.rs"]
mod developer_mode;
#[path = "bindings/effect.rs"]
mod effect;
#[path = "bindings/file_system.rs"]
mod file_system;
#[path = "bindings/git_identity.rs"]
mod git_identity;
#[path = "bindings/native.rs"]
mod native;
#[path = "bindings/plugin.rs"]
mod plugin;
#[path = "bindings/project.rs"]
mod project;
#[path = "bindings/proxy.rs"]
mod proxy;
#[path = "bindings/runtime_log_level.rs"]
mod runtime_log_level;
#[path = "bindings/session.rs"]
mod session;
#[path = "bindings/skill.rs"]
mod skill;
#[path = "bindings/skill_import.rs"]
mod skill_import;
#[path = "bindings/task.rs"]
mod task;
#[path = "bindings/workflow.rs"]
mod workflow;
#[path = "bindings/workflow_run.rs"]
mod workflow_run;
#[path = "bindings/workspace.rs"]
mod workspace;

/// Composes namespace-owned bindings only while exporting Desktop artifacts.
pub(crate) fn bindings() -> Vec<Binding> {
    [
        project::BINDINGS,
        developer_mode::BINDINGS,
        runtime_log_level::BINDINGS,
        task::BINDINGS,
        session::BINDINGS,
        agent_runtime::BINDINGS,
        app_events::BINDINGS,
        effect::BINDINGS,
        skill::BINDINGS,
        skill_import::BINDINGS,
        agent::BINDINGS,
        agent_import::BINDINGS,
        plugin::BINDINGS,
        proxy::BINDINGS,
        file_system::BINDINGS,
        git_identity::BINDINGS,
        workflow::BINDINGS,
        workflow_run::BINDINGS,
        workspace::BINDINGS,
        native::BINDINGS,
    ]
    .into_iter()
    .flat_map(|bindings| bindings.iter().copied())
    .collect()
}

/// Explicit grants distinguish the trusted application from isolated plugin Webviews.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Permission {
    MainWebview,
    MainAndPluginWebviews,
}

/// Binds supported operations and native commands without optional or inferred routing fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Binding {
    Unary {
        operation: &'static str,
        handler: &'static str,
        permission: Permission,
    },
    Stream {
        operation: &'static str,
        handler: &'static str,
    },
    Native {
        handler: &'static str,
        permission: Permission,
    },
}
