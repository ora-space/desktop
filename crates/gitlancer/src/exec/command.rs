use std::path::PathBuf;

use crate::exec::env::GitEnv;

/// Classifies the side-effect profile of a Git command so upper layers can apply policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitIntent {
    ReadOnly,
    Mutating,
    Network,
}

/// Represents one fully assembled Git CLI invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: GitEnv,
    pub intent: GitIntent,
}

impl GitCommand {
    /// Creates a command object so the execution layer can stay decoupled from use-case-specific builders.
    pub fn new(cwd: PathBuf, args: Vec<String>, env: GitEnv, intent: GitIntent) -> Self {
        Self {
            cwd,
            args,
            env,
            intent,
        }
    }
}
