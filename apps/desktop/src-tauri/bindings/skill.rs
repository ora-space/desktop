//! Desktop bindings for skill.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "createSkill",
        handler: "commands::skill::create_skill",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getSkill",
        handler: "commands::skill::get_skill",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listSkills",
        handler: "commands::skill::list_skills",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateSkill",
        handler: "commands::skill::update_skill",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteSkill",
        handler: "commands::skill::delete_skill",
        permission: Permission::MainWebview,
    },
];
