//! Desktop bindings for agent import.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "prepareAgentImport",
        handler: "commands::agent::prepare_agent_import",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "commitAgentImport",
        handler: "commands::agent::commit_agent_import",
        permission: Permission::MainWebview,
    },
];
