use std::path::PathBuf;

use ora_application::{
    ProjectRepository, ProjectRepositoryError, SessionRepository, SessionRepositoryError,
    TaskRepository, TaskRepositoryError, WorktreeRepository, WorktreeRepositoryError,
};
use ora_domain::{
    AuditFields, Project, ProjectId, Session, SessionId, SessionStatus, Task, TaskId, TaskStatus,
    Worktree, WorktreeActivity, WorktreeId,
};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteProjectRepository,
    SqliteSessionRepository, SqliteTaskRepository, SqliteWorktreeRepository, TimestampSource,
    default_migration_catalog,
};

/// Produces deterministic bootstrap timestamps so repository tests can assert stored objects.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource {
    now: i64,
}

impl TimestampSource for FixedTimestampSource {
    /// Returns the deterministic timestamp configured for the current test.
    fn current_timestamp_millis(&self) -> i64 {
        self.now
    }
}

/// Verifies pooled repository connections use the requested SQLite runtime settings.
#[test]
fn bootstrapped_repository_pool_configures_sqlite_pragmas() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();

    let (journal_mode, busy_timeout, synchronous) = pool
        .with_connection(|connection| {
            let journal_mode = connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?;
            let busy_timeout =
                connection.pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))?;
            let synchronous =
                connection.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?;

            Ok((journal_mode, busy_timeout, synchronous))
        })
        .unwrap();

    assert_eq!(journal_mode, "wal".to_string());
    assert_eq!(busy_timeout, 5_000_i64);
    assert_eq!(synchronous, 1_i64);
}

/// Verifies the SQLite-backed project repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn project_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let created_project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(10, 10, false),
    );

    assert_eq!(
        repository.create_project(created_project.clone()).unwrap(),
        created_project.clone()
    );
    assert_eq!(
        repository.find_project(&created_project.id).unwrap(),
        Some(created_project.clone())
    );
    assert_eq!(
        repository.list_projects().unwrap(),
        vec![created_project.clone()]
    );

    let updated_project = Project::new(
        created_project.id.clone(),
        "Ora Updated",
        "/tmp/ora-updated",
        AuditFields::new(10, 20, false),
    );

    assert_eq!(
        repository.update_project(updated_project.clone()).unwrap(),
        updated_project.clone()
    );
    assert_eq!(
        repository.find_project(&updated_project.id).unwrap(),
        Some(updated_project.clone())
    );
    assert_eq!(
        repository
            .soft_delete_project(&updated_project.id, /*deleted_at*/ 30)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_project(&updated_project.id).unwrap(), None);
    assert_eq!(repository.list_projects().unwrap(), Vec::<Project>::new());
}

/// Verifies the SQLite-backed task repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn task_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteTaskRepository::new(pool);
    let created_task = Task::new(
        TaskId::new("task-1"),
        ProjectId::new("project-1"),
        "Wire the pool",
        TaskStatus::Todo,
        Some(WorktreeId::new("worktree-1")),
        AuditFields::new(11, 11, false),
    );

    assert_eq!(
        repository.create_task(created_task.clone()).unwrap(),
        created_task.clone()
    );
    assert_eq!(
        repository.find_task(&created_task.id).unwrap(),
        Some(created_task.clone())
    );
    assert_eq!(repository.list_tasks().unwrap(), vec![created_task.clone()]);

    let updated_task = Task::new(
        created_task.id.clone(),
        created_task.project_id.clone(),
        "Wire the repository pool",
        TaskStatus::Doing,
        None,
        AuditFields::new(11, 21, false),
    );

    assert_eq!(
        repository.update_task(updated_task.clone()).unwrap(),
        updated_task.clone()
    );
    assert_eq!(
        repository.find_task(&updated_task.id).unwrap(),
        Some(updated_task.clone())
    );
    assert_eq!(
        repository
            .soft_delete_task(&updated_task.id, /*deleted_at*/ 31)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_task(&updated_task.id).unwrap(), None);
    assert_eq!(repository.list_tasks().unwrap(), Vec::<Task>::new());
}

/// Verifies the SQLite-backed session repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn session_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteSessionRepository::new(pool);
    let created_session = Session::new(
        SessionId::new("session-1"),
        TaskId::new("task-1"),
        "agent-1",
        Some("provider-1".to_string()),
        SessionStatus::Running,
        AuditFields::new(12, 12, false),
    );

    assert_eq!(
        repository.create_session(created_session.clone()).unwrap(),
        created_session.clone()
    );
    assert_eq!(
        repository.find_session(&created_session.id).unwrap(),
        Some(created_session.clone())
    );
    assert_eq!(
        repository.list_sessions().unwrap(),
        vec![created_session.clone()]
    );

    let updated_session = Session::new(
        created_session.id.clone(),
        created_session.task_id.clone(),
        "agent-2",
        None,
        SessionStatus::Stopped,
        AuditFields::new(12, 22, false),
    );

    assert_eq!(
        repository.update_session(updated_session.clone()).unwrap(),
        updated_session.clone()
    );
    assert_eq!(
        repository.find_session(&updated_session.id).unwrap(),
        Some(updated_session.clone())
    );
    assert_eq!(
        repository
            .soft_delete_session(&updated_session.id, /*deleted_at*/ 32)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_session(&updated_session.id).unwrap(), None);
    assert_eq!(repository.list_sessions().unwrap(), Vec::<Session>::new());
}

/// Verifies the SQLite-backed worktree repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn worktree_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorktreeRepository::new(pool);
    let created_worktree = Worktree::new(
        WorktreeId::new("worktree-1"),
        TaskId::new("task-1"),
        Some("feature/db-pool".to_string()),
        WorktreeActivity::Inactive,
        AuditFields::new(13, 13, false),
    );

    assert_eq!(
        repository
            .create_worktree(created_worktree.clone())
            .unwrap(),
        created_worktree.clone()
    );
    assert_eq!(
        repository.find_worktree(&created_worktree.id).unwrap(),
        Some(created_worktree.clone())
    );
    assert_eq!(
        repository.list_worktrees().unwrap(),
        vec![created_worktree.clone()]
    );

    let updated_worktree = Worktree::new(
        created_worktree.id.clone(),
        created_worktree.task_id.clone(),
        None,
        WorktreeActivity::Active,
        AuditFields::new(13, 23, false),
    );

    assert_eq!(
        repository
            .update_worktree(updated_worktree.clone())
            .unwrap(),
        updated_worktree.clone()
    );
    assert_eq!(
        repository.find_worktree(&updated_worktree.id).unwrap(),
        Some(updated_worktree.clone())
    );
    assert_eq!(
        repository
            .soft_delete_worktree(&updated_worktree.id, /*deleted_at*/ 33)
            .unwrap(),
        true
    );
    assert_eq!(
        repository.find_worktree(&updated_worktree.id).unwrap(),
        None
    );
    assert_eq!(repository.list_worktrees().unwrap(), Vec::<Worktree>::new());
}

/// Verifies a single repository pool can back all four application repository adapters together.
#[test]
fn repository_pool_composes_all_repository_adapters() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let task_repository = SqliteTaskRepository::new(pool.clone());
    let session_repository = SqliteSessionRepository::new(pool.clone());
    let worktree_repository = SqliteWorktreeRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(40, 40, false),
    );
    let task = Task::new(
        TaskId::new("task-1"),
        project.id.clone(),
        "Implement pool composition",
        TaskStatus::Todo,
        Some(WorktreeId::new("worktree-1")),
        AuditFields::new(41, 41, false),
    );
    let session = Session::new(
        SessionId::new("session-1"),
        task.id.clone(),
        "agent-1",
        Some("provider-1".to_string()),
        SessionStatus::Running,
        AuditFields::new(42, 42, false),
    );
    let worktree = Worktree::new(
        WorktreeId::new("worktree-1"),
        task.id.clone(),
        Some("feature/composition".to_string()),
        WorktreeActivity::Active,
        AuditFields::new(43, 43, false),
    );

    assert_eq!(
        project_repository.create_project(project.clone()).unwrap(),
        project.clone()
    );
    assert_eq!(
        task_repository.create_task(task.clone()).unwrap(),
        task.clone()
    );
    assert_eq!(
        session_repository.create_session(session.clone()).unwrap(),
        session.clone()
    );
    assert_eq!(
        worktree_repository
            .create_worktree(worktree.clone())
            .unwrap(),
        worktree.clone()
    );
    assert_eq!(
        project_repository.find_project(&project.id).unwrap(),
        Some(project)
    );
    assert_eq!(task_repository.find_task(&task.id).unwrap(), Some(task));
    assert_eq!(
        session_repository.find_session(&session.id).unwrap(),
        Some(session)
    );
    assert_eq!(
        worktree_repository.find_worktree(&worktree.id).unwrap(),
        Some(worktree)
    );
}

/// Verifies project repositories translate SQLite statement failures into application-owned errors.
#[test]
fn project_repository_reports_sqlite_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(50, 50, false),
    );

    repository.create_project(project.clone()).unwrap();

    assert_eq!(
        repository.create_project(project),
        Err(ProjectRepositoryError::OperationFailed(
            "sqlite error: UNIQUE constraint failed: projects.id".to_string(),
        ))
    );
}

/// Verifies task repositories translate invalid persisted status values into application-owned errors.
#[test]
fn task_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteTaskRepository::new(pool.clone());

    insert_invalid_task_row(&pool);

    assert_eq!(
        repository.find_task(&TaskId::new("task-invalid")),
        Err(TaskRepositoryError::OperationFailed(
            "domain model error: invalid task status value: 99".to_string(),
        ))
    );
}

/// Verifies session repositories translate invalid persisted status values into application-owned errors.
#[test]
fn session_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteSessionRepository::new(pool.clone());

    insert_invalid_session_row(&pool);

    assert_eq!(
        repository.find_session(&SessionId::new("session-invalid")),
        Err(SessionRepositoryError::OperationFailed(
            "domain model error: invalid session status value: 99".to_string(),
        ))
    );
}

/// Verifies worktree repositories translate invalid persisted activity values into application-owned errors.
#[test]
fn worktree_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorktreeRepository::new(pool.clone());

    insert_invalid_worktree_row(&pool);

    assert_eq!(
        repository.find_worktree(&WorktreeId::new("worktree-invalid")),
        Err(WorktreeRepositoryError::OperationFailed(
            "domain model error: invalid worktree activity value: 99".to_string(),
        ))
    );
}

/// Bootstraps a file-backed SQLite database and returns the ready repository pool.
fn bootstrapped_repository_pool() -> (TempDir, RepositoryPool) {
    let temp_dir = TempDir::new().unwrap();
    let pool = DatabaseBootstrapper::new(FixedTimestampSource {
        now: 1_700_000_000_000,
    })
    .bootstrap_repository_pool(
        &DatabaseLocation::path(database_path(&temp_dir)),
        &default_migration_catalog().unwrap(),
    )
    .unwrap();

    (temp_dir, pool)
}

/// Builds the file path used by a repository integration test database.
fn database_path(temp_dir: &TempDir) -> PathBuf {
    temp_dir.path().join("repository.sqlite3")
}

/// Inserts one task row with an invalid status integer for row-mapping error coverage.
fn insert_invalid_task_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO tasks (id, project_id, title, status, worktree_id, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "task-invalid",
                "project-1",
                "Broken task",
                99,
                Option::<String>::None,
                60,
                60,
                0,
            ],
        )?;

        Ok(())
    })
    .unwrap();
}

/// Inserts one session row with an invalid status integer for row-mapping error coverage.
fn insert_invalid_session_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO sessions (id, task_id, agent_id, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "session-invalid",
                "task-1",
                "agent-1",
                Option::<String>::None,
                99,
                61,
                61,
                0,
            ],
        )?;

        Ok(())
    })
    .unwrap();
}

/// Inserts one worktree row with an invalid activity integer for row-mapping error coverage.
fn insert_invalid_worktree_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO worktrees (id, task_id, branch_name, is_active, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "worktree-invalid",
                "task-1",
                Option::<String>::None,
                99,
                62,
                62,
                0,
            ],
        )?;

        Ok(())
    })
    .unwrap();
}
