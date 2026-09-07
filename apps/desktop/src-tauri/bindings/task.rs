//! Desktop bindings for task.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "createTask",
        handler: "commands::task::create_task",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getTask",
        handler: "commands::task::get_task",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listTasks",
        handler: "commands::task::list_tasks",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateTask",
        handler: "commands::task::update_task",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteTask",
        handler: "commands::task::delete_task",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getTaskWorkspace",
        handler: "commands::task::get_task_workspace",
        permission: Permission::MainWebview,
    },
];
