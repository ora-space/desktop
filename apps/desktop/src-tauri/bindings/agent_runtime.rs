//! Desktop bindings for agent runtime.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getAgentRuntimeStatus",
        handler: "commands::agent_runtime::get_agent_runtime_status",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listAgentModels",
        handler: "commands::agent_runtime::list_agent_models",
        permission: Permission::MainWebview,
    },
];
