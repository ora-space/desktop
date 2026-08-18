//! Integration tests for durable Git cleanup bookkeeping: cascade job
//! registration, the task-workspace commit unit of work, and lease lifecycle.

use crate::{
    CascadeDeleteOutcome, DatabaseBootstrapper, DatabaseLocation, RepositoryPool,
    SqliteCascadeRepository, SqliteGitCleanupJobRepository, SqliteTaskWorkspaceRepository,
    SqliteWorkflowRunRepository, SqliteWorktreeProvisioningLeaseRepository,
    default_migration_catalog,
};
use ora_application::{
    DeleteWorkflowRunResult, TaskWorkspaceCommit, WorkflowRunRepository, WorkspaceCommitOutcome,
    WorktreeProvisioningLeaseStore,
};
use ora_domain::{
    AuditFields, GitCleanupJobState, ProjectId, SessionStatus, Task, TaskId, Worktree,
    WorktreeActivity, WorktreeBaseline, WorktreeId, WorktreeProvisioningLease,
    WorktreeProvisioningLeaseId,
};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

/// Builds an isolated SQLite pool for one test.
fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
    let temp_dir = TempDir::new().expect("create temp dir");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("catalog"),
        )
        .expect("bootstrap pool");
    (temp_dir, pool)
}

/// Inserts one project with a worktree-backed task and one project-root task.
fn insert_mixed_project_fixture(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute_batch(
            "INSERT INTO projects VALUES ('project-1', 'Ora', '/repos/project-1', 1, 1, 0);
             INSERT INTO tasks (id, project_id, title, worktree_id, created_at, updated_at, is_deleted)
             VALUES ('task-1', 'project-1', 'Worktree task', 'worktree-1', 1, 1, 0);
             INSERT INTO tasks (id, project_id, title, worktree_id, created_at, updated_at, is_deleted)
             VALUES ('task-2', 'project-1', 'Project-root task', NULL, 1, 1, 0);
             INSERT INTO worktrees (
                 id, task_id, branch_name, checkout_root, is_active, created_at, updated_at, is_deleted, base_commit_id
             ) VALUES ('worktree-1', 'task-1', 'ora/task-1', '/worktrees/task-1', 1, 1, 1, 0, 'base-commit');
             INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES ('session-1', 'task-1', 'ora-space.opencode', 'provider-1', 1, 1, 1, 0);",
        )?;
        Ok(())
    })
    .unwrap();
}

/// Reduces persisted jobs into their comparable identity tuples.
fn job_identities(
    pool: &RepositoryPool,
) -> Vec<(
    String,
    String,
    String,
    Option<String>,
    String,
    GitCleanupJobState,
)> {
    SqliteGitCleanupJobRepository::new(pool.clone())
        .list_jobs()
        .expect("list jobs")
        .into_iter()
        .map(|job| {
            (
                job.project_id.to_string(),
                job.task_id.to_string(),
                job.repository_root,
                job.checkout_root,
                job.branch_name,
                job.state,
            )
        })
        .collect()
}

/// Verifies task deletion registers exactly one job with the persisted identity.
#[test]
fn task_cascade_registers_one_cleanup_job() {
    let (_temp, pool) = bootstrapped_pool();
    insert_mixed_project_fixture(&pool);

    assert_eq!(
        SqliteCascadeRepository::new(pool.clone())
            .delete_task(&TaskId::new("task-1"), 100)
            .unwrap(),
        CascadeDeleteOutcome::Deleted
    );

    assert_eq!(
        job_identities(&pool),
        vec![(
            "project-1".to_string(),
            "task-1".to_string(),
            "/repos/project-1".to_string(),
            Some("/worktrees/task-1".to_string()),
            "ora/task-1".to_string(),
            GitCleanupJobState::Pending,
        )]
    );
}

/// Verifies project deletion produces jobs only for worktree-backed descendants.
#[test]
fn project_cascade_skips_project_root_tasks() {
    let (_temp, pool) = bootstrapped_pool();
    insert_mixed_project_fixture(&pool);

    assert_eq!(
        SqliteCascadeRepository::new(pool.clone())
            .delete_project(&ProjectId::new("project-1"), 100)
            .unwrap(),
        CascadeDeleteOutcome::Deleted
    );

    assert_eq!(
        job_identities(&pool),
        vec![(
            "project-1".to_string(),
            "task-1".to_string(),
            "/repos/project-1".to_string(),
            Some("/worktrees/task-1".to_string()),
            "ora/task-1".to_string(),
            GitCleanupJobState::Pending,
        )]
    );
}

/// Verifies a rejected deletion (running session) registers no job at all.
#[test]
fn rejected_deletion_registers_no_job() {
    let (_temp, pool) = bootstrapped_pool();
    insert_mixed_project_fixture(&pool);
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE sessions SET status = ?1 WHERE id = 'session-1'",
            rusqlite::params![SessionStatus::Running.database_value()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        SqliteCascadeRepository::new(pool.clone())
            .delete_project(&ProjectId::new("project-1"), 100)
            .unwrap(),
        CascadeDeleteOutcome::ActiveSession
    );
    assert_eq!(job_identities(&pool), vec![]);
}

/// Builds the task/worktree pair the workspace commit tests persist.
fn workspace_rows() -> (Task, Worktree) {
    let worktree = Worktree::new(
        WorktreeId::new("worktree-1"),
        TaskId::new("task-1"),
        Some("ora/task-1".to_string()),
        Some("/worktrees/task-1".to_string()),
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(1, 1, false),
    );
    let task = Task::new(
        TaskId::new("task-1"),
        ProjectId::new("project-1"),
        "Worktree task",
        Some(worktree.id.clone()),
        AuditFields::new(1, 1, false),
    );
    (task, worktree)
}

/// Builds one lease matching the workspace fixture rows.
fn workspace_lease() -> WorktreeProvisioningLease {
    WorktreeProvisioningLease::new(
        WorktreeProvisioningLeaseId::new("lease-1"),
        ProjectId::new("project-1"),
        TaskId::new("task-1"),
        "/repos/project-1",
        "/worktrees/task-1",
        "ora/task-1",
        /*lease_expires_at*/ 600_000,
        /*now*/ 1,
    )
}

/// Verifies the unit of work persists both rows and consumes the lease atomically.
#[test]
fn workspace_commit_persists_rows_and_consumes_lease() {
    let (_temp, pool) = bootstrapped_pool();
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO projects VALUES ('project-1', 'Ora', '/repos/project-1', 1, 1, 0)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let lease_repository = SqliteWorktreeProvisioningLeaseRepository::new(pool.clone());
    lease_repository.create_lease(&workspace_lease()).unwrap();
    let (task, worktree) = workspace_rows();

    assert_eq!(
        SqliteTaskWorkspaceRepository::new(pool.clone())
            .commit_worktree_task(
                &task,
                &worktree,
                &WorktreeProvisioningLeaseId::new("lease-1")
            )
            .unwrap(),
        WorkspaceCommitOutcome::Committed
    );

    assert_eq!(lease_repository.list_leases().unwrap(), vec![]);
    let (task_count, worktree_checkout) = pool
        .with_connection(|connection| {
            Ok((
                connection.query_row(
                    "SELECT COUNT(*) FROM tasks WHERE id = 'task-1' AND is_deleted = 0",
                    [],
                    |row| row.get::<_, i64>(0),
                )?,
                connection.query_row(
                    "SELECT checkout_root FROM worktrees WHERE id = 'worktree-1'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(
        (task_count, worktree_checkout),
        (1, Some("/worktrees/task-1".to_string()))
    );
}

/// Verifies a deleted project rejects the commit and persists nothing.
#[test]
fn workspace_commit_rejects_invisible_project() {
    let (_temp, pool) = bootstrapped_pool();
    let lease_repository = SqliteWorktreeProvisioningLeaseRepository::new(pool.clone());
    lease_repository.create_lease(&workspace_lease()).unwrap();
    let (task, worktree) = workspace_rows();
    let repository = SqliteTaskWorkspaceRepository::new(pool.clone());

    assert_eq!(
        repository
            .commit_worktree_task(
                &task,
                &worktree,
                &WorktreeProvisioningLeaseId::new("lease-1")
            )
            .unwrap(),
        WorkspaceCommitOutcome::ProjectNotVisible
    );
    assert_eq!(
        repository.commit_project_root_task(&task).unwrap(),
        WorkspaceCommitOutcome::ProjectNotVisible
    );

    // Nothing was persisted and the lease still owns the provisioned resources.
    let task_count = pool
        .with_connection(|connection| {
            connection
                .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                .map_err(Into::into)
        })
        .unwrap();
    assert_eq!(task_count, 0);
    assert_eq!(lease_repository.list_leases().unwrap().len(), 1);
}

/// Verifies renewal extends only live leases and reports absence truthfully.
#[test]
fn lease_renewal_reports_liveness() {
    let (_temp, pool) = bootstrapped_pool();
    let repository = SqliteWorktreeProvisioningLeaseRepository::new(pool.clone());
    repository.create_lease(&workspace_lease()).unwrap();
    let lease_id = WorktreeProvisioningLeaseId::new("lease-1");

    assert!(
        WorktreeProvisioningLeaseStore::renew_lease(&repository, &lease_id, 900_000, 2).unwrap()
    );
    assert!(repository.delete_lease(&lease_id).unwrap());
    assert!(
        !WorktreeProvisioningLeaseStore::renew_lease(&repository, &lease_id, 900_000, 3).unwrap()
    );
}

/// Verifies releasing an already-consumed lease is a harmless no-op.
#[test]
fn releasing_a_missing_lease_creates_no_job() {
    let (_temp, pool) = bootstrapped_pool();
    let repository = SqliteWorktreeProvisioningLeaseRepository::new(pool.clone());

    repository
        .release_to_cleanup(&WorktreeProvisioningLeaseId::new("lease-gone"), 10)
        .unwrap();

    assert_eq!(job_identities(&pool), vec![]);
}

/// Verifies workflow-run deletion registers the run-task's cleanup job in-cascade.
#[test]
fn workflow_run_delete_registers_cleanup_job() {
    let (_temp, pool) = bootstrapped_pool();
    pool.with_connection(|connection| {
        connection.execute_batch(
            "INSERT INTO projects VALUES ('project-1', 'Ora', '/repos/project-1', 1, 1, 0);
             INSERT INTO workflows (id, name, published_snapshot_id, created_at, updated_at, is_deleted)
             VALUES ('workflow-a', 'Workflow', NULL, 1, 1, 0);
             INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at, updated_at, is_deleted)
             VALUES ('snapshot-a', 'workflow-a', 'v1', '{}', 1, 1, 0);
             INSERT INTO workflow_runs (id, workflow_id, snapshot_id, run_status, state, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted)
             VALUES ('run-1', 'workflow-a', 'snapshot-a', 3, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1, 1, 0);
             INSERT INTO tasks (id, project_id, title, type, workflow_run_id, worktree_id, created_at, updated_at, is_deleted)
             VALUES ('task-1', 'project-1', 'Run task', 1, 'run-1', 'worktree-1', 1, 1, 0);
             INSERT INTO worktrees (
                 id, task_id, branch_name, checkout_root, is_active, created_at, updated_at, is_deleted, base_commit_id
             ) VALUES ('worktree-1', 'task-1', 'ora/task-1', '/worktrees/task-1', 1, 1, 1, 0, 'base-commit');",
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        SqliteWorkflowRunRepository::new(pool.clone())
            .soft_delete_run(&ora_domain::WorkflowRunId::new("run-1"), 100)
            .unwrap(),
        DeleteWorkflowRunResult::Deleted
    );

    assert_eq!(
        job_identities(&pool),
        vec![(
            "project-1".to_string(),
            "task-1".to_string(),
            "/repos/project-1".to_string(),
            Some("/worktrees/task-1".to_string()),
            "ora/task-1".to_string(),
            GitCleanupJobState::Pending,
        )]
    );
}

/// Verifies the dispatch query respects due time, ordering, and the batch limit.
#[test]
fn due_jobs_respect_schedule_and_limit() {
    let (_temp, pool) = bootstrapped_pool();
    let repository = SqliteGitCleanupJobRepository::new(pool.clone());
    for (id, next_attempt_at) in [("job-a", 10), ("job-b", 5), ("job-c", 100)] {
        let mut job = ora_domain::GitCleanupJob::pending(
            ora_domain::GitCleanupJobId::new(id),
            ProjectId::new("project-1"),
            TaskId::new(id),
            "/repos/project-1",
            None,
            "ora/branch",
            1,
        );
        job.next_attempt_at = next_attempt_at;
        repository.insert_job(&job).unwrap();
    }

    let due = repository.due_jobs(/*now*/ 50, /*limit*/ 1).unwrap();
    assert_eq!(
        due.iter().map(|job| job.id.to_string()).collect::<Vec<_>>(),
        vec!["job-b".to_string()]
    );
    let due = repository.due_jobs(50, 10).unwrap();
    assert_eq!(
        due.iter().map(|job| job.id.to_string()).collect::<Vec<_>>(),
        vec!["job-b".to_string(), "job-a".to_string()]
    );
}

/// Verifies completed jobs are purged only after their retention cutoff.
#[test]
fn purges_only_expired_completed_jobs() {
    let (_temp, pool) = bootstrapped_pool();
    let repository = SqliteGitCleanupJobRepository::new(pool.clone());
    for id in ["job-old", "job-new", "job-pending"] {
        let job = ora_domain::GitCleanupJob::pending(
            ora_domain::GitCleanupJobId::new(id),
            ProjectId::new("project-1"),
            TaskId::new(id),
            "/repos/project-1",
            None,
            "ora/branch",
            1,
        );
        repository.insert_job(&job).unwrap();
    }
    repository
        .mark_completed(&ora_domain::GitCleanupJobId::new("job-old"), 100)
        .unwrap();
    repository
        .mark_completed(&ora_domain::GitCleanupJobId::new("job-new"), 900)
        .unwrap();

    assert_eq!(repository.purge_completed_before(500).unwrap(), 1);
    assert_eq!(
        repository
            .list_jobs()
            .unwrap()
            .iter()
            .map(|job| job.id.to_string())
            .collect::<Vec<_>>(),
        vec!["job-new".to_string(), "job-pending".to_string()]
    );
}
