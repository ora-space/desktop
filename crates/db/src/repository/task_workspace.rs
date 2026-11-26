use crate::{LocalTimestampSource, TimestampSource};
use ora_application::{
    RepositoryError, ScheduledRunWorktreeLink, TaskWorkspaceCommit, WorkspaceCommitOutcome,
};
use ora_domain::{
    Task, Workspace, WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation,
    WorkspaceProvisionerKind, WorkspaceProvisioningState, Worktree, WorktreeProvisioningLeaseId,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::repository::RepositoryPool;
use crate::repository::connection::bool_to_sqlite;

/// Commits task creation atomically against concurrent project deletion.
///
/// The task and worktree rows, the project visibility check, and the
/// provisioning lease removal all share one immediate transaction, so a
/// project cascade can never interleave between them: either the cascade sees
/// the committed rows (and registers cleanup jobs for them), or this commit
/// fails with `ProjectNotVisible` and the caller compensates the provisioned
/// Git resources itself.
#[derive(Clone, Debug)]
pub struct SqliteTaskWorkspaceRepository<Clock = LocalTimestampSource> {
    pool: RepositoryPool,
    clock: Clock,
}

impl SqliteTaskWorkspaceRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self {
            pool,
            clock: LocalTimestampSource,
        }
    }
}

impl<Clock: TimestampSource> SqliteTaskWorkspaceRepository<Clock> {
    /// Injects the clock used for scheduled-run timestamps inside immediate transactions.
    pub fn with_clock(pool: RepositoryPool, clock: Clock) -> Self {
        Self { pool, clock }
    }
}

impl<Clock: TimestampSource> TaskWorkspaceCommit for SqliteTaskWorkspaceRepository<Clock> {
    /// Atomically persists a worktree-backed task and releases its provisioning lease.
    fn commit_worktree_task(
        &self,
        task: &Task,
        worktree: &Worktree,
        lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                if !project_visible(&transaction, task.project_id.as_ref())? {
                    return Ok(WorkspaceCommitOutcome::ProjectNotVisible);
                }
                commit_worktree_task_rows(&transaction, task, worktree, lease_id)?;
                transaction.commit()?;
                Ok(WorkspaceCommitOutcome::Committed)
            })
            .map_err(RepositoryError::new)
    }

    /// Atomically persists a worktree-backed task, binds the Scheduled Run's
    /// worktree ownership, and releases its provisioning lease.
    ///
    /// The ownership UPDATE runs inside the same immediate transaction as the
    /// task rows, and only matches a run that is still `accepted` for this
    /// project and not bound yet: a run the startup recovery already settled, a
    /// conflicting project, or a concurrent double commit rolls the whole task
    /// creation back so the lease keeps ownership for its compensation paths.
    fn commit_worktree_task_for_scheduled_run(
        &self,
        task: &Task,
        worktree: &Worktree,
        lease_id: &WorktreeProvisioningLeaseId,
        run_link: &ScheduledRunWorktreeLink,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                if !project_visible(&transaction, task.project_id.as_ref())? {
                    return Ok(WorkspaceCommitOutcome::ProjectNotVisible);
                }
                let updated_at = self.clock.current_timestamp_millis();
                let bound = transaction.execute(
                    "UPDATE scheduled_runs
                     SET worktree_task_id = ?1, worktree_path = ?2, updated_at = ?3
                     WHERE id = ?4 AND status = 'accepted' AND project_id = ?5
                       AND worktree_task_id IS NULL",
                    params![
                        task.id.as_ref(),
                        run_link.worktree_path,
                        updated_at,
                        run_link.run_id.as_ref(),
                        task.project_id.as_ref(),
                    ],
                )?;
                if bound != 1 {
                    return Ok(WorkspaceCommitOutcome::RunNotAdmissible);
                }
                commit_worktree_task_rows(&transaction, task, worktree, lease_id)?;
                transaction.commit()?;
                Ok(WorkspaceCommitOutcome::Committed)
            })
            .map_err(RepositoryError::new)
    }
}

/// Persists the workspace, provisioning, worktree, and task rows, then removes
/// the provisioning lease — the shared tail of both commit paths, all inside
/// the caller's open immediate transaction.
fn commit_worktree_task_rows(
    transaction: &Transaction<'_>,
    task: &Task,
    worktree: &Worktree,
    lease_id: &WorktreeProvisioningLeaseId,
) -> Result<(), crate::DatabaseError> {
    let checkout_root = transaction.query_row(
        "SELECT checkout_root FROM worktree_provisioning_leases WHERE id = ?1",
        params![lease_id.as_ref()],
        |row| row.get::<_, String>(0),
    )?;
    let workspace = Workspace::new(
        task.workspace_id.clone(),
        task.project_id.clone(),
        WorkspaceKind::Isolated,
        WorkspaceLocation::local_filesystem(checkout_root),
        WorkspaceLifecycle::Active,
        task.audit_fields.clone(),
    );
    insert_workspace(transaction, &workspace)?;
    insert_provisioning(transaction, &workspace)?;
    insert_worktree(transaction, worktree)?;
    insert_task(transaction, task)?;
    transaction.execute(
        "DELETE FROM worktree_provisioning_leases WHERE id = ?1",
        params![lease_id.as_ref()],
    )?;
    Ok(())
}

/// Reports whether the owning project row is still visible to new descendants.
fn project_visible(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<bool, rusqlite::Error> {
    Ok(transaction
        .query_row(
            "SELECT 1 FROM projects WHERE id = ?1 AND is_deleted = 0",
            params![project_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Inserts one task row inside the open commit transaction.
fn insert_task(transaction: &Transaction<'_>, task: &Task) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO tasks (id, workspace_id, title, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            task.id.as_ref(),
            task.workspace_id.as_ref(),
            &task.title,
            task.audit_fields.created_at,
            task.audit_fields.updated_at,
            bool_to_sqlite(task.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Inserts one worktree row inside the open commit transaction.
fn insert_worktree(
    transaction: &Transaction<'_>,
    worktree: &Worktree,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO worktrees (workspace_id, branch_name, base_commit_id, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            worktree.workspace_id.as_ref(),
            worktree.branch_name.as_deref(),
            worktree.baseline.commit_id(),
            worktree.audit_fields.created_at,
            worktree.audit_fields.updated_at,
            bool_to_sqlite(worktree.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Inserts the workspace identity before its worktree and task projection so all runtime records
/// can reference the workspace directly after this transaction commits.
fn insert_workspace(
    transaction: &Transaction<'_>,
    workspace: &Workspace,
) -> Result<(), crate::DatabaseError> {
    let location_id = format!("{}-location", workspace.id);
    let locator = match &workspace.location {
        WorkspaceLocation::LocalFilesystem { path } => {
            serde_json::json!({ "path": path }).to_string()
        }
        WorkspaceLocation::Ssh {
            connection_ref,
            path,
        } => serde_json::json!({ "connection_ref": connection_ref, "path": path }).to_string(),
        WorkspaceLocation::RemoteTarget {
            target_ref,
            locator,
            ..
        } => serde_json::json!({ "target_ref": target_ref, "locator": locator }).to_string(),
    };
    transaction.execute(
        "INSERT INTO workspace_locations (id, location_kind, plugin_id, locator_version, locator_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)",
        params![
            location_id,
            workspace.location.database_kind(),
            match &workspace.location {
                WorkspaceLocation::RemoteTarget { plugin_id, .. } => Some(plugin_id),
                WorkspaceLocation::LocalFilesystem { .. } | WorkspaceLocation::Ssh { .. } => None,
            },
            locator,
            workspace.audit_fields.created_at,
            workspace.audit_fields.updated_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            workspace.id.as_ref(),
            workspace.project_id.as_ref(),
            workspace.kind.database_value(),
            format!("{}-location", workspace.id),
            workspace.lifecycle.database_value(),
            workspace.audit_fields.created_at,
            workspace.audit_fields.updated_at,
            bool_to_sqlite(workspace.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Records a ready local Git provisioning result for an already-created isolated workspace.
fn insert_provisioning(
    transaction: &Transaction<'_>,
    workspace: &Workspace,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO workspace_provisioning (workspace_id, provisioner_kind, state, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            workspace.id.as_ref(),
            WorkspaceProvisionerKind::LocalGit.database_value(),
            WorkspaceProvisioningState::Ready.database_value(),
            workspace.audit_fields.created_at,
            workspace.audit_fields.updated_at,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod scheduled_ownership_tests {
    use super::SqliteTaskWorkspaceRepository;
    use crate::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteProjectRepository,
        SqliteScheduledRunRepository, SqliteScheduledTaskRepository,
        SqliteWorktreeProvisioningLeaseRepository, default_migration_catalog,
        test_clock::TestClock,
    };
    use ora_application::{
        ProjectRepository, ScheduledOccurrenceCandidate, ScheduledOccurrenceRepository,
        ScheduledRunLifecycleRepository, ScheduledTaskRepository, TaskWorkspaceCommit,
        WorkspaceCommitOutcome,
    };
    use ora_domain::{
        AgentRef, AuditFields, Project, ProjectId, ScheduledOccurrence, ScheduledRun,
        ScheduledRunFailureReason, ScheduledRunId, ScheduledRunTransition, ScheduledTask,
        ScheduledTaskApproval, ScheduledTaskExecutionLocation, ScheduledTaskId,
        ScheduledTaskSchedule, Task, TaskId, WorkspaceId, WorkspaceLocation, Worktree,
        WorktreeActivity, WorktreeBaseline, WorktreeProvisioningLease, WorktreeProvisioningLeaseId,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
        let temp_dir = TempDir::new().expect("create temp directory");
        let database_path = temp_dir.path().join("task-workspace.sqlite3");
        let pool = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap database");
        (temp_dir, pool)
    }

    fn seed_project(pool: &RepositoryPool, temp_root: &std::path::Path, project_id: &str) {
        let workspace_root = temp_root.join(project_id);
        std::fs::create_dir_all(&workspace_root).expect("create project workspace");
        SqliteProjectRepository::new(pool.clone())
            .create_project(
                Project::new(
                    ProjectId::new(project_id),
                    "Provisioning project",
                    AuditFields::new(10, 10, /*is_deleted*/ false),
                ),
                WorkspaceLocation::local_filesystem(workspace_root.to_string_lossy().into_owned()),
            )
            .expect("create project");
    }

    fn seed_admitted_run(pool: &RepositoryPool, run_id: &str) -> ScheduledRunId {
        let task = ScheduledTask::new(
            ScheduledTaskId::new("scheduled-task-1"),
            "Quality report",
            true,
            ScheduledTaskSchedule::every_hours(4).expect("hourly schedule"),
            ScheduledTaskExecutionLocation::Local,
            ProjectId::new("project-1"),
            AgentRef::parse("ora/agent").expect("agent ref"),
            Some("default".to_string()),
            "Run the quality gate",
            Some(ScheduledTaskApproval::full_auto_in_worktree(10)),
            AuditFields::new(10, 10, /*is_deleted*/ false),
        )
        .expect("task");
        SqliteScheduledTaskRepository::new(pool.clone())
            .create_scheduled_task(task.clone())
            .expect("create Scheduled Task");
        let run = ScheduledRun::accepted(
            ScheduledRunId::new(run_id),
            ScheduledOccurrence {
                task_id: task.id.clone(),
                scheduled_at: 1_000,
            },
            &task,
            1_001,
        );
        SqliteScheduledRunRepository::new(pool.clone())
            .resolve_scheduled_occurrence(ScheduledOccurrenceCandidate::Run(Box::new(run.clone())))
            .expect("admit run");
        run.id
    }

    fn seed_lease(pool: &RepositoryPool, project_id: &str, workspace_id: &str, lease_id: &str) {
        let lease = WorktreeProvisioningLease::new(
            WorktreeProvisioningLeaseId::new(lease_id),
            ProjectId::new(project_id),
            WorkspaceId::new(workspace_id),
            "repository-root",
            format!("worktrees/{workspace_id}"),
            format!("ora/{workspace_id}"),
            /*lease_expires_at*/ 60_000,
            /*now*/ 10,
        );
        SqliteWorktreeProvisioningLeaseRepository::new(pool.clone())
            .create_lease(&lease)
            .expect("create lease");
    }

    fn committed_task(workspace_id: &str) -> Task {
        Task::new(
            TaskId::new(format!("task-{workspace_id}")),
            ProjectId::new("project-1"),
            WorkspaceId::new(workspace_id),
            "Scheduled run run-1",
            AuditFields::new(10, 10, /*is_deleted*/ false),
        )
    }

    fn committed_worktree(workspace_id: &str) -> Worktree {
        Worktree::new(
            WorkspaceId::new(workspace_id),
            Some(format!("ora/{workspace_id}")),
            WorktreeBaseline::recorded("base-commit").expect("baseline"),
            WorktreeActivity::Active,
            AuditFields::new(10, 10, /*is_deleted*/ false),
        )
    }

    fn run_binding(pool: &RepositoryPool, run_id: &str) -> (Option<String>, Option<String>) {
        pool.with_connection(|connection| {
            let (task, path) = connection.query_row(
                "SELECT worktree_task_id, worktree_path FROM scheduled_runs WHERE id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )?;
            Ok((task, path))
        })
        .expect("run row")
    }

    fn lease_count(pool: &RepositoryPool, lease_id: &str) -> i64 {
        pool.with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT COUNT(*) FROM worktree_provisioning_leases WHERE id = ?1",
                [lease_id],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .expect("lease probe")
    }

    fn task_count(pool: &RepositoryPool) -> i64 {
        pool.with_connection(|connection| {
            Ok(connection
                .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))?)
        })
        .expect("task count")
    }

    #[test]
    fn binds_the_run_and_persists_the_task_in_one_commit() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        let run_id = seed_admitted_run(&pool, "run-1");
        seed_lease(&pool, "project-1", "workspace-task-1", "lease-1");

        let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-1"),
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            )
            .expect("ownership commit");

        assert_eq!(outcome, WorkspaceCommitOutcome::Committed);
        assert_eq!(
            run_binding(&pool, run_id.as_ref()),
            (
                Some("task-workspace-task-1".to_string()),
                Some("worktrees/workspace-task-1".to_string())
            )
        );
        assert_eq!(lease_count(&pool, "lease-1"), 0);
    }

    #[test]
    fn committed_ownership_survives_reopen() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        let run_id = seed_admitted_run(&pool, "run-1");
        seed_lease(&pool, "project-1", "workspace-task-1", "lease-1");

        let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-1"),
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            )
            .expect("ownership commit");
        assert_eq!(outcome, WorkspaceCommitOutcome::Committed);

        // Reopen the database: the acknowledged commit must still show the run
        // binding, the persisted task rows, and no provisioning lease.
        let database_path = temp_dir.path().join("task-workspace.sqlite3");
        drop(pool);
        let reopened = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("reopen database");
        assert_eq!(
            run_binding(&reopened, run_id.as_ref()),
            (
                Some("task-workspace-task-1".to_string()),
                Some("worktrees/workspace-task-1".to_string())
            ),
            "the ownership binding survives reopen"
        );
        assert_eq!(
            lease_count(&reopened, "lease-1"),
            0,
            "no lease remains after reopen"
        );
        assert_eq!(task_count(&reopened), 1, "the task row survives reopen");
    }

    #[test]
    fn rejects_a_settled_run_and_keeps_the_lease() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        let run_id = seed_admitted_run(&pool, "run-1");
        // Startup recovery settled the run while its worktree task provisioned.
        SqliteScheduledRunRepository::new(pool.clone())
            .transition_scheduled_run(
                &run_id,
                ScheduledRunTransition::Fail {
                    reason: ScheduledRunFailureReason::ExecutionError,
                },
                2_000,
            )
            .expect("settle run");
        seed_lease(&pool, "project-1", "workspace-task-1", "lease-1");

        let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-1"),
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            )
            .expect("inadmissible commit");

        assert_eq!(outcome, WorkspaceCommitOutcome::RunNotAdmissible);
        // Nothing committed and nothing compensated away: the lease keeps the
        // Git resources for its durable cleanup path.
        assert_eq!(lease_count(&pool, "lease-1"), 1);
        assert_eq!(task_count(&pool), 0);
    }

    #[test]
    fn rejects_a_second_binding_for_the_same_run() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        let run_id = seed_admitted_run(&pool, "run-1");
        seed_lease(&pool, "project-1", "workspace-task-1", "lease-1");
        let repository =
            SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10));

        let first = repository
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-1"),
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            )
            .expect("first commit");
        assert_eq!(first, WorkspaceCommitOutcome::Committed);

        seed_lease(&pool, "project-1", "workspace-task-2", "lease-2");
        let second = repository
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-2"),
                &committed_worktree("workspace-task-2"),
                &WorktreeProvisioningLeaseId::new("lease-2"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-2".to_string(),
                },
            )
            .expect("second commit");

        assert_eq!(second, WorkspaceCommitOutcome::RunNotAdmissible);
        // The replay must not create a second task nor rebind the run.
        assert_eq!(task_count(&pool), 1);
        assert_eq!(
            run_binding(&pool, run_id.as_ref()).0,
            Some("task-workspace-task-1".to_string())
        );
        assert_eq!(lease_count(&pool, "lease-2"), 1);
    }

    #[test]
    fn rejects_a_run_from_another_project() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        seed_project(&pool, temp_dir.path(), "project-2");
        let run_id = seed_admitted_run(&pool, "run-1");
        seed_lease(&pool, "project-2", "workspace-task-1", "lease-1");

        // The task belongs to project-2 while the admitted run belongs to
        // project-1: the ownership predicate must refuse the binding.
        let mismatched = Task::new(
            TaskId::new("task-workspace-task-1"),
            ProjectId::new("project-2"),
            WorkspaceId::new("workspace-task-1"),
            "Scheduled run run-1",
            AuditFields::new(10, 10, /*is_deleted*/ false),
        );
        let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
            .commit_worktree_task_for_scheduled_run(
                &mismatched,
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            )
            .expect("mismatched commit");

        assert_eq!(outcome, WorkspaceCommitOutcome::RunNotAdmissible);
        assert_eq!(run_binding(&pool, run_id.as_ref()).0, None);
        assert_eq!(lease_count(&pool, "lease-1"), 1);
    }

    #[test]
    fn lease_delete_failure_rolls_back_the_whole_commit_and_keeps_the_lease() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        let run_id = seed_admitted_run(&pool, "run-1");
        seed_lease(&pool, "project-1", "workspace-task-1", "lease-1");

        // Inject a real SQLite failure at the lease-removal step: the whole
        // immediate transaction must roll back, so no task rows surface and the
        // lease keeps ownership for its durable cleanup path.
        pool.with_connection(|connection| {
            connection
                .execute(
                    "CREATE TRIGGER fail_lease_delete BEFORE DELETE ON worktree_provisioning_leases
                     BEGIN SELECT RAISE(ABORT, 'injected lease delete failure'); END;",
                    [],
                )
                .expect("install delete trigger");
            Ok(())
        })
        .expect("install delete trigger");

        let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-1"),
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            );

        assert!(outcome.is_err(), "the injected DELETE failure surfaces");
        assert_eq!(task_count(&pool), 0, "the commit rolls back, no task rows");
        assert_eq!(
            run_binding(&pool, run_id.as_ref()).0,
            None,
            "the run ownership write rolls back too"
        );
        assert_eq!(
            lease_count(&pool, "lease-1"),
            1,
            "the lease survives for compensation"
        );

        // Reopen the database: the durable state must match the rolled-back view.
        let database_path = temp_dir.path().join("task-workspace.sqlite3");
        drop(pool);
        let reopened = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("reopen database");
        assert_eq!(task_count(&reopened), 0, "no task rows after reopen");
        assert_eq!(
            run_binding(&reopened, run_id.as_ref()).0,
            None,
            "no run binding after reopen"
        );
        assert_eq!(
            lease_count(&reopened, "lease-1"),
            1,
            "the lease survives reopen"
        );
    }

    #[test]
    fn project_deletion_during_commit_reports_not_visible_and_keeps_the_lease() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path(), "project-1");
        let run_id = seed_admitted_run(&pool, "run-1");
        seed_lease(&pool, "project-1", "workspace-task-1", "lease-1");
        // The project disappears while Git work and the commit race it.
        ora_application::ProjectRepository::soft_delete_project(
            &SqliteProjectRepository::new(pool.clone()),
            &ProjectId::new("project-1"),
            20,
        )
        .expect("soft-delete the project");

        let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
            .commit_worktree_task_for_scheduled_run(
                &committed_task("workspace-task-1"),
                &committed_worktree("workspace-task-1"),
                &WorktreeProvisioningLeaseId::new("lease-1"),
                &ora_application::ScheduledRunWorktreeLink {
                    run_id: run_id.clone(),
                    worktree_path: "worktrees/workspace-task-1".to_string(),
                },
            )
            .expect("concurrent project deletion");

        assert_eq!(outcome, WorkspaceCommitOutcome::ProjectNotVisible);
        assert_eq!(
            task_count(&pool),
            0,
            "no task rows when the project is gone"
        );
        assert_eq!(
            lease_count(&pool, "lease-1"),
            1,
            "the lease still owns the Git resources"
        );
    }
}
