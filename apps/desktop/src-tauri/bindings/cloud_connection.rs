//! Desktop bindings for the Cloud connection. Only the trusted main Webview may sign in or out.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getCloudConnection",
        handler: "commands::cloud_connection::get_cloud_connection",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "setCloudServer",
        handler: "commands::cloud_connection::set_cloud_server",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "startCloudLogin",
        handler: "commands::cloud_connection::start_cloud_login",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "cancelCloudLogin",
        handler: "commands::cloud_connection::cancel_cloud_login",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "logoutCloud",
        handler: "commands::cloud_connection::logout_cloud",
        permission: Permission::MainWebview,
    },
];
