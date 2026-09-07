//! Backend composition and runtime adapters for workflow runs.

mod api;
mod engine;
mod executor;
pub(crate) mod interactive;
mod operations;
mod prerequisites;
mod prompt;
mod recovery;
#[cfg(test)]
mod test_fixture;
mod worktree;

pub(crate) use engine::build_workflow_run_engine;
pub(crate) use operations::WorkflowRunSetup;
pub use operations::WorkflowRuns;
pub(crate) use recovery::{prune_orphaned_baselines, run_workflow_run_boot_sweep};
