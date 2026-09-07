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
];
