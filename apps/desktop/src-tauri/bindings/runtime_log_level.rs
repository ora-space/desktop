//! Desktop bindings for runtime log level.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getRuntimeLogLevel",
        handler: "commands::settings::get_runtime_log_level",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "setRuntimeLogLevel",
        handler: "commands::settings::set_runtime_log_level",
        permission: Permission::MainWebview,
    },
];
