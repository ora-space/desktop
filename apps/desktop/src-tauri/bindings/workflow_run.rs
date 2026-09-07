//! Desktop bindings for workflow run.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "createWorkflowRun",
        handler: "commands::workflow_run::create_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getWorkflowRun",
        handler: "commands::workflow_run::get_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listWorkflowRuns",
        handler: "commands::workflow_run::list_workflow_runs",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listWorkflowRunsByWorkflow",
        handler: "commands::workflow_run::list_workflow_runs_by_workflow",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listWorkflowNodeRuns",
        handler: "commands::workflow_run::list_workflow_node_runs",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteWorkflowRun",
        handler: "commands::workflow_run::delete_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "renameWorkflowRun",
        handler: "commands::workflow_run::rename_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "startWorkflowRun",
        handler: "commands::workflow_run::start_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "cancelWorkflowRun",
        handler: "commands::workflow_run::cancel_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "restartWorkflowRun",
        handler: "commands::workflow_run::restart_workflow_run",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateWorkflowRunInput",
        handler: "commands::workflow_run::update_workflow_run_input",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "completeWorkflowNode",
        handler: "commands::workflow_run::complete_workflow_node",
        permission: Permission::MainWebview,
    },
];
