//! Desktop bindings for file system.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "listWorkspaceDirectory",
        handler: "commands::files::list_workspace_directory",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "readWorkspaceFile",
        handler: "commands::files::read_workspace_file",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "searchWorkspace",
        handler: "commands::files::search_workspace",
        permission: Permission::MainWebview,
    },
    Binding::Stream {
        operation: "watchWorkspace",
        handler: "commands::files::start_workspace_watch",
    },
    Binding::Unary {
        operation: "listProjectDirectory",
        handler: "commands::files::list_project_directory",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "readProjectFile",
        handler: "commands::files::read_project_file",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "searchProject",
        handler: "commands::files::search_project",
        permission: Permission::MainWebview,
    },
    Binding::Stream {
        operation: "watchProject",
        handler: "commands::files::start_project_watch",
    },
    Binding::Unary {
        operation: "createWorkspaceEntry",
        handler: "commands::files::create_workspace_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "createProjectEntry",
        handler: "commands::files::create_project_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "copyWorkspaceEntry",
        handler: "commands::files::copy_workspace_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "copyProjectEntry",
        handler: "commands::files::copy_project_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "moveWorkspaceEntry",
        handler: "commands::files::move_workspace_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "moveProjectEntry",
        handler: "commands::files::move_project_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteWorkspaceEntry",
        handler: "commands::files::delete_workspace_entry",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteProjectEntry",
        handler: "commands::files::delete_project_entry",
        permission: Permission::MainWebview,
    },
];
