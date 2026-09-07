//! Desktop bindings for workflow.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "createWorkflow",
        handler: "commands::workflow::create_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getWorkflow",
        handler: "commands::workflow::get_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listWorkflows",
        handler: "commands::workflow::list_workflows",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateWorkflow",
        handler: "commands::workflow::update_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteWorkflow",
        handler: "commands::workflow::delete_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getDraft",
        handler: "commands::workflow::get_workflow_draft",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateDraft",
        handler: "commands::workflow::update_workflow_draft",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "publishWorkflow",
        handler: "commands::workflow::publish_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "rollbackWorkflow",
        handler: "commands::workflow::rollback_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "activateWorkflow",
        handler: "commands::workflow::activate_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listVersions",
        handler: "commands::workflow::list_workflow_versions",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getVersion",
        handler: "commands::workflow::get_workflow_version",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteSnapshot",
        handler: "commands::workflow::delete_workflow_snapshot",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getWorkflowSnapshot",
        handler: "commands::workflow::get_workflow_snapshot",
        permission: Permission::MainWebview,
    },
];
