//! Desktop bindings for native.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Native {
        handler: "commands::stream::stream_contract",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "commands::stream::cancel_contract_stream",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "commands::workspace::get_worktree_root",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "commands::workspace::set_worktree_root",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "commands::workspace::resolve_task_cwd",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "commands::workspace::resolve_workspace_cwd",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "open_location::open_location",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "open_external::open_external_url",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "commands::workflow::write_workflow_export",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "diagnostic_logs::download_today_log",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_capabilities",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_list",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_open",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_close",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_set_bounds",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_set_visible",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_popout",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_dock",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_reload",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::workbench_bridge::plugin_webview_invoke",
        permission: Permission::MainAndPluginWebviews,
    },
    Binding::Native {
        handler: "surface::commands::surface_resolve_download",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "surface::commands::surface_discard_download",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "update::commands::get_desktop_update_status",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "update::commands::install_desktop_update",
        permission: Permission::MainWebview,
    },
    Binding::Native {
        handler: "update::commands::check_desktop_update",
        permission: Permission::MainWebview,
    },
];
