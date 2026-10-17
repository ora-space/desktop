use std::path::PathBuf;
use std::time::Duration;

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
    /// How long this command may run, when its caller has a tighter or looser limit than the
    /// runner's budget for its intent; runners without time budgets ignore it.
    pub time_limit: Option<Duration>,
}

impl GitCommand {
    /// Creates a command object so the execution layer can stay decoupled from use-case-specific builders.
    pub fn new(cwd: PathBuf, args: Vec<String>, env: GitEnv, intent: GitIntent) -> Self {
        Self {
            cwd,
            args,
            env,
            intent,
            time_limit: None,
        }
    }

    /// Bounds this command by `limit` instead of the runner's budget for its intent.
    pub fn with_time_limit(mut self, limit: Duration) -> Self {
        self.time_limit = Some(limit);
        self
    }
}
