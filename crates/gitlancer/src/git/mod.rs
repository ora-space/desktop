pub mod branch;
pub mod commit;
pub mod repository;
pub mod status;
pub mod worktree;

use crate::exec::runner::GitRunner;

/// Represents the typed Git runtime that Ora and tests can drive through an injected runner.
#[derive(Debug, Clone)]
pub struct Git<R: GitRunner> {
    runner: R,
}

impl<R: GitRunner> Git<R> {
    /// Creates a Git runtime from one execution strategy so all use cases share the same operational contract.
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    /// Exposes the injected runner to submodules so they can build use cases without re-owning execution state.
    pub fn runner(&self) -> &R {
        &self.runner
    }
}
