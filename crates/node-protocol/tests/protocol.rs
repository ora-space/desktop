//! Public codec contract tests, grouped by the business that owns each rule.

#[path = "protocol/agent_session.rs"]
mod agent_session;
#[path = "protocol/execution.rs"]
mod execution;
#[path = "protocol/framing.rs"]
mod framing;
#[path = "protocol/plugin.rs"]
mod plugin;
#[path = "protocol/repository.rs"]
mod repository;
#[path = "protocol/repository_results.rs"]
mod repository_results;
#[path = "protocol/revision.rs"]
mod revision;
#[path = "protocol/revision_restore.rs"]
mod revision_restore;
#[path = "protocol/session.rs"]
mod session;
#[path = "protocol/support.rs"]
mod support;
#[path = "protocol/worktree.rs"]
mod worktree;
