use ora_domain::{GitCleanupJob, GitCleanupJobId, GitCleanupJobState, ProjectId, WorkspaceId};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteGitCleanupJobRepository,
    TimestampSource, default_migration_catalog,
};

/// Supplies deterministic timestamps for the cleanup repository fixture.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource;

impl TimestampSource for FixedTimestampSource {
    /// Returns the fixed timestamp used while opening the test database.
    fn current_timestamp_millis(&self) -> i64 {
        1
    }
}

/// Verifies cleanup jobs persist workspace identity independently of user-facing tasks.
#[test]
fn persists_cleanup_jobs_by_workspace() {
    let (_temp_dir, pool) = bootstrapped_pool();
    let repository = SqliteGitCleanupJobRepository::new(pool);
    let job = GitCleanupJob::pending(
        GitCleanupJobId::new("cleanup-1"),
        ProjectId::new("project-1"),
        WorkspaceId::new("workspace-1"),
        "/repo",
        Some("/worktree".to_string()),
        "ora/task-1",
        10,
    );

    repository.insert_job(&job).expect("insert cleanup job");
    assert_eq!(
        repository.list_jobs().expect("list cleanup jobs"),
        vec![job.clone()]
    );
    assert_eq!(
        repository.due_jobs(10, 10).expect("list due jobs"),
        vec![job.clone()]
    );

    repository
        .mark_completed(&GitCleanupJobId::new("cleanup-1"), 20)
        .expect("complete cleanup job");
    let mut completed = job;
    completed.state = GitCleanupJobState::Completed;
    completed.last_attempt_at = Some(20);
    completed.updated_at = 20;
    assert_eq!(
        repository.list_jobs().expect("reload cleanup job"),
        vec![completed]
    );
}

/// Opens a file-backed repository so pooled adapters exercise the same path as production.
fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
    let temp_dir = TempDir::new().expect("create temporary database directory");
    let database_path = temp_dir.path().join("cleanup.sqlite3");
    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap repository pool")
    });
    (temp_dir, pool)
}
