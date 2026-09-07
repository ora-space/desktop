//! Desktop bindings for skill import.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "prepareSkillImport",
        handler: "commands::skill::prepare_skill_import",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getSkillImport",
        handler: "commands::skill::get_skill_import",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "commitSkillImport",
        handler: "commands::skill::commit_skill_import",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "cancelSkillImport",
        handler: "commands::skill::cancel_skill_import",
        permission: Permission::MainWebview,
    },
];
