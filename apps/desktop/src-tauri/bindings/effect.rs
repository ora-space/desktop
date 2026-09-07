//! Desktop bindings for effect.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[Binding::Unary {
    operation: "getEffectTargetStatus",
    handler: "commands::effect::get_effect_target_status",
    permission: Permission::MainWebview,
}];
