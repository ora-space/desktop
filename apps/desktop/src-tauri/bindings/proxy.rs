//! Desktop bindings for proxy.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getProxySettings",
        handler: "commands::settings::get_proxy_settings",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "setProxySettings",
        handler: "commands::settings::set_proxy_settings",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "clearProxySettings",
        handler: "commands::settings::clear_proxy_settings",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "checkProxySettings",
        handler: "commands::settings::check_proxy_settings",
        permission: Permission::MainWebview,
    },
];
