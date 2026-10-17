//! Desktop bindings for effect.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "getEffectTargetStatus",
        handler: "commands::effect::get_effect_target_status",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "recoverEffectTarget",
        handler: "commands::effect::recover_effect_target",
        permission: Permission::MainWebview,
    },
];
