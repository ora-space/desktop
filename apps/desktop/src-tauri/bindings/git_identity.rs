//! Desktop bindings for git identity.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[Binding::Unary {
    operation: "getGitIdentity",
    handler: "commands::git_identity::get_git_identity",
    permission: Permission::MainWebview,
}];
