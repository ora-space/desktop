use std::fs;
use std::sync::Arc;

use ora_application::{
    ProjectRepository, ScheduledArtifactStore, ScheduledOccurrenceCandidate,
    ScheduledOccurrenceRepository, ScheduledRunArtifactRepository, ScheduledRunLifecycleRepository,
    ScheduledTaskHistoryRepository, ScheduledTaskRepository, SweepScheduledRunArtifactsHandler,
};
use ora_domain::{
    AgentRef, AuditFields, Project, ProjectId, ScheduledArtifactKind, ScheduledArtifactRejection,
    ScheduledArtifactRejectionReason, ScheduledOccurrence, ScheduledRun, ScheduledRunArtifact,
    ScheduledRunFailureReason, ScheduledRunId, ScheduledRunTransition, ScheduledTask,
    ScheduledTaskApproval, ScheduledTaskExecutionLocation, ScheduledTaskId, ScheduledTaskSchedule,
    SessionId, TaskId, WorkspaceLocation,
};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog,
    repository::{
        SqliteProjectRepository, SqliteScheduledRunRepository, SqliteScheduledTaskRepository,
    },
    test_clock::TestClock,
};

fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
    // The artifact store deliberately rejects symlinked ancestors, and the system temporary
    // root is not guaranteed to be free of them, so keep these fixtures under the checkout
    // instead of relaxing the production safety boundary.
    let temporary = tempfile::tempdir_in(std::env::current_dir().expect("current directory"))
        .expect("create temp directory");
    let pool = bootstrapped_pool_at(&temporary);
    (temporary, pool)
}

fn bootstrapped_pool_at(temporary: &TempDir) -> RepositoryPool {
    let database_path = temporary.path().join("scheduled-artifact.sqlite3");
    DatabaseBootstrapper::new(TestClock::new(1))
        .bootstrap_repository_pool(
            &DatabaseLocation::path(database_path),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap database")
}

fn artifact(name: &str, size: u64, registered_at: i64) -> ScheduledRunArtifact {
    ScheduledRunArtifact {
        relative_path: name.to_string(),
        kind: ScheduledArtifactKind::Markdown,
        size_bytes: size,
        registered_at,
    }
}

#[test]
fn records_lists_finds_and_deletes_a_run_manifest() {
    let (_temporary, pool) = bootstrapped_pool();
    let repository = SqliteScheduledRunRepository::new(pool);
    let run_id = ScheduledRunId::new("run-1");
    repository
        .record_scheduled_run_artifacts(
            &run_id,
            &[
                artifact("first.md", 5, 10),
                artifact("nested/second.json", 6, 11),
            ],
            &[ScheduledArtifactRejection {
                name: "secret.pdf".to_string(),
                reason: ScheduledArtifactRejectionReason::UnsupportedType,
            }],
            10,
        )
        .expect("record artifacts");

    assert_eq!(
        repository
            .list_scheduled_run_artifacts(&run_id)
            .expect("list artifacts"),
        vec![
            artifact("first.md", 5, 10),
            artifact("nested/second.json", 6, 11)
        ]
    );
    assert_eq!(
        repository
            .find_scheduled_run_artifact(&run_id, "nested/second.json")
            .expect("find artifact"),
        Some(artifact("nested/second.json", 6, 11))
    );
    assert_eq!(
        repository
            .list_scheduled_run_artifact_rejections(&run_id)
            .expect("list rejections"),
        vec![ScheduledArtifactRejection {
            name: "secret.pdf".to_string(),
            reason: ScheduledArtifactRejectionReason::UnsupportedType,
        }]
    );
    assert!(
        repository
            .delete_scheduled_run_artifact(&run_id, "first.md")
            .expect("delete artifact")
    );
    assert_eq!(
        repository
            .list_scheduled_run_artifacts(&run_id)
            .expect("list after delete"),
        vec![artifact("nested/second.json", 6, 11)]
    );
    assert_eq!(
        repository
            .delete_scheduled_run_artifact_rows(&run_id)
            .expect("delete rows"),
        2
    );
    assert!(
        repository
            .find_scheduled_artifact_run(&run_id)
            .expect("find subject")
            .is_none()
    );
}

#[test]
fn pruned_runs_surface_as_missing_so_the_sweep_can_remove_their_artifacts() {
    let (_temporary, pool) = bootstrapped_pool();
    let repository = SqliteScheduledRunRepository::new(pool);
    let run_id = ScheduledRunId::new("run-pruned");
    repository
        .record_scheduled_run_artifacts(&run_id, &[artifact("report.md", 6, 1)], &[], 1)
        .expect("record artifacts");

    assert_eq!(
        repository
            .find_scheduled_artifact_run(&run_id)
            .expect("find subject"),
        Some(ora_application::ScheduledArtifactRunRecord {
            run_id: run_id.clone(),
            task_id: None,
            updated_at: None,
            run_missing: true,
            task_deleted: true,
            retains_worktree: false,
        })
    );
    assert_eq!(
        repository
            .list_scheduled_artifact_runs()
            .expect("list artifact runs"),
        vec![ora_application::ScheduledArtifactRunRecord {
            run_id,
            task_id: None,
            updated_at: None,
            run_missing: true,
            task_deleted: true,
            retains_worktree: false,
        }]
    );
}

/// Seeds a visible project and task so run and artifact joins resolve.
fn seed_task(pool: &RepositoryPool, temporary: &TempDir) -> ScheduledTask {
    let workspace_root = temporary.path().join("project");
    fs::create_dir_all(&workspace_root).expect("create project root");
    SqliteProjectRepository::new(pool.clone())
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Project",
                AuditFields::new(1, 1, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_root.to_string_lossy()),
        )
        .expect("create project");
    let mut task = ScheduledTask::new(
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
        AuditFields::new(10, 10, false),
    )
    .expect("create task");
    task.freeze_worktree_base_branch("main");
    SqliteScheduledTaskRepository::new(pool.clone())
        .create_scheduled_task(task.clone())
        .expect("persist task");
    task
}

/// Seeds a visible task and one succeeded run linked to a retained worktree.
fn seed_task_and_run(
    pool: &RepositoryPool,
    temporary: &TempDir,
    run_id: &ScheduledRunId,
) -> ScheduledTaskId {
    let task = seed_task(pool, temporary);
    let runs = SqliteScheduledRunRepository::new(pool.clone());
    let run = ScheduledRun::accepted(
        run_id.clone(),
        ScheduledOccurrence {
            task_id: task.id.clone(),
            scheduled_at: 100,
        },
        &task,
        101,
    );
    runs.resolve_scheduled_occurrence(ScheduledOccurrenceCandidate::run(run))
        .expect("admit run");
    runs.transition_scheduled_run(
        run_id,
        ScheduledRunTransition::Start {
            session_id: SessionId::new("session-1"),
            worktree_path: "worktrees/run-1".to_string(),
            worktree_task_id: TaskId::new("worktree-task-1"),
        },
        102,
    )
    .expect("start run");
    runs.transition_scheduled_run(run_id, ScheduledRunTransition::Succeed, 103)
        .expect("succeed run");
    task.id
}

/// Stores one artifact file and its manifest row.
fn seed_artifact(
    store: &ScheduledArtifactStore,
    repository: &SqliteScheduledRunRepository,
    run_id: &ScheduledRunId,
) {
    let run_directory = store.root().join(run_id.as_ref());
    fs::create_dir_all(&run_directory).expect("create run directory");
    fs::write(run_directory.join("report.md"), "report").expect("write artifact");
    repository
        .record_scheduled_run_artifacts(run_id, &[artifact("report.md", 6, 1)], &[], 1)
        .expect("record artifact");
}

fn sweep(pool: &RepositoryPool, store: &ScheduledArtifactStore) {
    let repository = Arc::new(SqliteScheduledRunRepository::new(pool.clone()));
    SweepScheduledRunArtifactsHandler::new(repository, store.clone())
        .handle()
        .expect("sweep artifacts");
}

#[test]
fn artifacts_survive_a_repository_restart() {
    let (temporary, pool) = bootstrapped_pool();
    let run_id = ScheduledRunId::new("run-1");
    seed_task_and_run(&pool, &temporary, &run_id);
    let repository = SqliteScheduledRunRepository::new(pool.clone());
    seed_artifact(
        &ScheduledArtifactStore::new(temporary.path().join("artifacts")),
        &repository,
        &run_id,
    );
    drop(repository);
    drop(pool);

    let reopened = SqliteScheduledRunRepository::new(bootstrapped_pool_at(&temporary));
    assert_eq!(
        reopened
            .list_scheduled_run_artifacts(&run_id)
            .expect("read artifacts after restart"),
        vec![artifact("report.md", 6, 1)]
    );
}

#[test]
fn sweep_removes_artifacts_after_the_task_is_soft_deleted() {
    let (temporary, pool) = bootstrapped_pool();
    let run_id = ScheduledRunId::new("run-1");
    let task_id = seed_task_and_run(&pool, &temporary, &run_id);
    let store = ScheduledArtifactStore::new(temporary.path().join("artifacts"));
    let repository = SqliteScheduledRunRepository::new(pool.clone());
    seed_artifact(&store, &repository, &run_id);
    SqliteScheduledTaskRepository::new(pool.clone())
        .soft_delete_scheduled_task(&task_id, 200)
        .expect("soft-delete task");

    sweep(&pool, &store);

    assert!(
        repository
            .list_scheduled_run_artifacts(&run_id)
            .expect("list artifacts")
            .is_empty()
    );
    assert!(!store.root().join(run_id.as_ref()).exists());
}

#[test]
fn sweep_keeps_artifacts_for_a_disabled_task() {
    let (temporary, pool) = bootstrapped_pool();
    let run_id = ScheduledRunId::new("run-1");
    let task_id = seed_task_and_run(&pool, &temporary, &run_id);
    let store = ScheduledArtifactStore::new(temporary.path().join("artifacts"));
    let repository = SqliteScheduledRunRepository::new(pool.clone());
    seed_artifact(&store, &repository, &run_id);
    pool.with_connection_mut(|connection| {
        connection.execute(
            "UPDATE scheduled_tasks SET enabled = 0 WHERE id = ?1",
            [task_id.as_ref()],
        )?;
        Ok(())
    })
    .expect("disable task");

    sweep(&pool, &store);

    assert_eq!(
        repository
            .list_scheduled_run_artifacts(&run_id)
            .expect("list artifacts"),
        vec![artifact("report.md", 6, 1)]
    );
    assert!(
        store
            .root()
            .join(run_id.as_ref())
            .join("report.md")
            .is_file()
    );
}

#[test]
fn sweep_removes_artifacts_after_the_run_row_is_pruned() {
    let (temporary, pool) = bootstrapped_pool();
    let run_id = ScheduledRunId::new("run-1");
    seed_task_and_run(&pool, &temporary, &run_id);
    let store = ScheduledArtifactStore::new(temporary.path().join("artifacts"));
    let repository = SqliteScheduledRunRepository::new(pool.clone());
    seed_artifact(&store, &repository, &run_id);
    pool.with_connection_mut(|connection| {
        connection.execute(
            "DELETE FROM scheduled_runs WHERE id = ?1",
            [run_id.as_ref()],
        )?;
        Ok(())
    })
    .expect("prune run row");

    sweep(&pool, &store);

    assert!(
        repository
            .list_scheduled_run_artifacts(&run_id)
            .expect("list artifacts")
            .is_empty()
    );
    assert!(!store.root().join(run_id.as_ref()).exists());
}

/// Admits one occurrence and immediately fails it without a retained worktree.
fn admit_failed_run(
    repository: &SqliteScheduledRunRepository,
    task: &ScheduledTask,
    run_id: &ScheduledRunId,
    scheduled_at: i64,
) {
    let run = ScheduledRun::accepted(
        run_id.clone(),
        ScheduledOccurrence {
            task_id: task.id.clone(),
            scheduled_at,
        },
        task,
        scheduled_at + 1_000,
    );
    repository
        .resolve_scheduled_occurrence(ScheduledOccurrenceCandidate::run(run))
        .expect("admit run");
    repository
        .transition_scheduled_run(
            run_id,
            ScheduledRunTransition::Fail {
                reason: ScheduledRunFailureReason::ExecutionError,
            },
            scheduled_at + 2_000,
        )
        .expect("fail run");
}

#[test]
fn sweeping_removes_artifacts_pruned_by_the_history_bound() {
    let (temporary, pool) = bootstrapped_pool();
    let task = seed_task(&pool, &temporary);
    let store = ScheduledArtifactStore::new(temporary.path().join("artifacts"));
    let repository = SqliteScheduledRunRepository::new(pool.clone());
    let pruned_run = ScheduledRunId::new("run-0");
    admit_failed_run(&repository, &task, &pruned_run, 1);
    seed_artifact(&store, &repository, &pruned_run);
    for index in 1_i64..=100 {
        admit_failed_run(
            &repository,
            &task,
            &ScheduledRunId::new(format!("run-{index}")),
            index + 1,
        );
    }

    let history = repository
        .list_scheduled_task_history(&task.id)
        .expect("read history");
    assert!(
        !history.iter().any(|entry| matches!(
            entry,
            ora_domain::ScheduledTaskHistoryEntry::Run(run) if run.id == pruned_run
        )),
        "the 101st occurrence prunes the oldest run row"
    );

    sweep(&pool, &store);

    assert!(
        repository
            .list_scheduled_run_artifacts(&pruned_run)
            .expect("list pruned artifacts")
            .is_empty()
    );
    assert!(!store.root().join(pruned_run.as_ref()).exists());
}
