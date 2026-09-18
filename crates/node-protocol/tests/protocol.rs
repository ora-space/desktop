//! Public codec contract tests, grouped by the business that owns each rule.

#[path = "protocol/execution.rs"]
mod execution;
#[path = "protocol/framing.rs"]
mod framing;
#[path = "protocol/repository.rs"]
mod repository;
#[path = "protocol/repository_results.rs"]
mod repository_results;
#[path = "protocol/session.rs"]
mod session;
#[path = "protocol/support.rs"]
mod support;
#[path = "protocol/worktree.rs"]
mod worktree;
