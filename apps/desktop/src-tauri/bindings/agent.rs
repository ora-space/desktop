//! Desktop bindings for agent.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "createAgent",
        handler: "commands::agent::create_agent",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getAgent",
        handler: "commands::agent::get_agent",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listAgents",
        handler: "commands::agent::list_agents",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateAgent",
        handler: "commands::agent::update_agent",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteAgent",
        handler: "commands::agent::delete_agent",
        permission: Permission::MainWebview,
    },
];
