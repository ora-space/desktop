//! Desktop bindings for agent runtime.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getAgentRuntimeStatus",
        handler: "commands::agent_runtime::get_agent_runtime_status",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "discoverAgent",
        handler: "commands::agent_runtime::discover_agent",
        permission: Permission::MainWebview,
    },
];
