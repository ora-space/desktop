//! Backend composition and runtime adapters for workflow runs.

mod api;
mod checkpoint;
mod diagnosis;
mod engine;
mod executor;
pub(crate) mod interactive;
#[cfg(test)]
mod iteration_tests;
mod last_failure;
#[cfg(test)]
mod loop_resume_tests;
#[cfg(test)]
mod mixed_scope_tests;
mod operations;
mod prerequisites;
mod prompt;
mod recovery;
#[cfg(test)]
mod resume_gap_tests;
#[cfg(test)]
mod resume_tests;
mod rollback;
#[cfg(test)]
mod rollback_content_tests;
#[cfg(test)]
mod rollback_tests;
mod snapshot_switch;
#[cfg(test)]
mod snapshot_switch_tests;
#[cfg(test)]
mod test_fixture;
mod transitions;
mod worktree;

pub(crate) use engine::build_workflow_run_engine;
pub(crate) use operations::WorkflowRunSetup;
pub use operations::WorkflowRuns;
pub(crate) use recovery::{prune_orphaned_baselines, run_workflow_run_boot_sweep};
