//! Desktop bindings for project.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "createProject",
        handler: "commands::project::create_project",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getProject",
        handler: "commands::project::get_project",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listProjects",
        handler: "commands::project::list_projects",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listProjectBranches",
        handler: "commands::project::list_project_branches",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateProject",
        handler: "commands::project::update_project",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteProject",
        handler: "commands::project::delete_project",
        permission: Permission::MainWebview,
    },
];
