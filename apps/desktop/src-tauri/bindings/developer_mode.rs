//! Desktop bindings for developer mode.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getDeveloperMode",
        handler: "commands::settings::get_developer_mode",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "setDeveloperMode",
        handler: "commands::settings::set_developer_mode",
        permission: Permission::MainWebview,
    },
];
