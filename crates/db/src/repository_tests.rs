use std::path::PathBuf;

use ora_application::{
    AgentDefinitionRepository, ProjectRepository, ProjectRepositoryError,
    ProjectWorkContextRepository, SessionRepository, SessionRepositoryError,
    SkillImportCommitError, SkillImportUnitOfWork, SkillRepository, TaskRepository,
    TaskRepositoryError, WorktreeRepository, WorktreeRepositoryError,
};
use ora_domain::{
    AgentCli, AgentDefinition, AgentDefinitionId, AuditFields, Project, ProjectId,
    ProjectWorkContext, ProjectWorkContextId, ProjectWorkContextSurface, Session, SessionId,
    SessionStatus, Skill, SkillId, Task, TaskId, TaskStatus, Worktree, WorktreeActivity,
    WorktreeId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    CascadeDeleteOutcome, DatabaseBootstrapper, DatabaseLocation, RepositoryPool,
    SqliteAgentDefinitionRepository, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteProjectWorkContextRepository, SqliteSessionRepository, SqliteSkillImportUnitOfWork,
    SqliteSkillRepository, SqliteTaskRepository, SqliteWorktreeRepository, TimestampSource,
    default_migration_catalog,
};

/// Verifies catalog repositories use stable identifiers and hide soft-deleted rows.
#[test]
fn catalog_repositories_support_id_based_crud_and_allow_duplicate_names() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let skill_repository = SqliteSkillRepository::new(pool.clone());
    let agent_repository = SqliteAgentDefinitionRepository::new(pool);
    let created_skill = skill("skill-1", "review", "Reviews changes", 1, 1, false);
    let created_agent = agent("agent-1", "opencode", "OpenCode", 1, 1, false);

    assert_eq!(
        skill_repository
            .create_skill(created_skill.clone())
            .unwrap(),
        created_skill.clone()
    );
    assert_eq!(
        agent_repository
            .create_agent_definition(created_agent.clone())
            .unwrap(),
        created_agent.clone()
    );
    let earlier_skill = skill("skill-0", "review", "Builds", 0, 0, false);
    let earlier_agent = agent("agent-0", "opencode", "Assists", 0, 0, false);
    skill_repository
        .create_skill(earlier_skill.clone())
        .unwrap();
    agent_repository
        .create_agent_definition(earlier_agent.clone())
        .unwrap();
    assert_eq!(
        skill_repository.list_skills().unwrap(),
        vec![earlier_skill.clone(), created_skill.clone()]
    );
    assert_eq!(
        agent_repository.list_agent_definitions().unwrap(),
        vec![earlier_agent.clone(), created_agent.clone()]
    );
    let renamed_skill = skill("skill-1", "reviewer", "Reviews code", 1, 2, false);
    let renamed_agent = agent("agent-1", "reviewer-agent", "Reviews code", 1, 2, false);
    assert_eq!(
        skill_repository
            .update_skill(renamed_skill.clone())
            .unwrap(),
        renamed_skill.clone()
    );
    assert_eq!(
        agent_repository
            .update_agent_definition(renamed_agent.clone())
            .unwrap(),
        renamed_agent.clone()
    );
    assert_eq!(
        skill_repository
            .soft_delete_skill(&SkillId::new("skill-1"), 3)
            .unwrap(),
        true
    );
    assert_eq!(
        agent_repository
            .soft_delete_agent_definition(&AgentDefinitionId::new("agent-1"), 3)
            .unwrap(),
        true
    );
    assert_eq!(
        skill_repository
            .find_skill(&SkillId::new("skill-1"))
            .unwrap(),
        None
    );
    assert_eq!(
        agent_repository
            .find_agent_definition(&AgentDefinitionId::new("agent-1"))
            .unwrap(),
        None
    );
    assert_eq!(
        skill_repository
            .soft_delete_skill(&SkillId::new("missing"), 4)
            .unwrap(),
        false
    );
    assert_eq!(
        agent_repository
            .soft_delete_agent_definition(&AgentDefinitionId::new("missing"), 4)
            .unwrap(),
        false
    );
}

/// Verifies the import unit of work commits the row only when the promote callback succeeds.
#[test]
fn skill_import_unit_of_work_commits_on_success_and_rolls_back_on_promote_failure() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let unit_of_work = SqliteSkillImportUnitOfWork::new(pool.clone());
    let repository = SqliteSkillRepository::new(pool);
    let committed = skill("skill-commit", "grilling", "Grill", 1, 1, false);
    let rolled_back = skill("skill-rollback", "probe", "Probe", 2, 2, false);

    unit_of_work
        .insert_then(committed.clone(), || Ok(()))
        .unwrap();
    let rollback = unit_of_work
        .insert_then(rolled_back, || Err("promote failed".to_string()))
        .unwrap_err();

    assert_eq!(
        rollback,
        SkillImportCommitError::Promote {
            message: "promote failed".to_string(),
        }
    );
    assert_eq!(
        repository
            .find_skill(&SkillId::new("skill-commit"))
            .unwrap(),
        Some(committed)
    );
    assert_eq!(
        repository
            .find_skill(&SkillId::new("skill-rollback"))
            .unwrap(),
        None
    );
}

fn skill(
    id: &str,
    name: &str,
    description: &str,
    created_at: i64,
    updated_at: i64,
    is_deleted: bool,
) -> Skill {
    Skill::new(
        SkillId::new(id),
        name,
        description,
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
}

fn agent(
    id: &str,
    name: &str,
    description: &str,
    created_at: i64,
    updated_at: i64,
    is_deleted: bool,
) -> AgentDefinition {
    AgentDefinition::new(
        AgentDefinitionId::new(id),
        name,
        description,
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
}

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
        repository
            .find_project_by_name(&created_project.name)
            .unwrap(),
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
            .find_project_by_name(&updated_project.name)
            .unwrap(),
        Some(updated_project.clone())
    );
    assert_eq!(
        repository
            .soft_delete_project(&updated_project.id, /*deleted_at*/ 30)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_project(&updated_project.id).unwrap(), None);
    assert_eq!(
        repository
            .find_project_by_name(&updated_project.name)
            .unwrap(),
        None
    );
    assert_eq!(repository.list_projects().unwrap(), Vec::<Project>::new());
}

/// Verifies the SQLite-backed project repository can load one visible project by exact name.
#[test]
fn project_repository_finds_visible_project_by_name() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(14, 14, false),
    );

    repository.create_project(project.clone()).unwrap();

    assert_eq!(
        repository.find_project_by_name("Ora").unwrap(),
        Some(project)
    );
    assert_eq!(repository.find_project_by_name("Missing").unwrap(), None);
}

/// Verifies the SQLite-backed project repository hides soft-deleted rows during name-based lookup.
#[test]
fn project_repository_ignores_soft_deleted_projects_during_name_lookup() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(15, 15, false),
    );

    repository.create_project(project.clone()).unwrap();
    repository
        .soft_delete_project(&project.id, /*deleted_at*/ 16)
        .unwrap();

    assert_eq!(repository.find_project_by_name("Ora").unwrap(), None);
}

/// Verifies the SQLite-backed project work context repository preserves lease-aware rows and cleanup.
#[test]
fn project_work_context_repository_supports_active_lookup_and_cleanup() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectWorkContextRepository::new(pool);
    let created_context = ProjectWorkContext::new(
        ProjectWorkContextId::new("context-1"),
        ProjectWorkContextSurface::Tauri,
        "window-1",
        ProjectId::new("project-1"),
        120,
        10,
        10,
    );

    assert_eq!(
        repository
            .create_project_work_context(created_context.clone())
            .unwrap(),
        created_context.clone()
    );
    assert_eq!(
        repository
            .find_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
            .unwrap(),
        Some(created_context.clone())
    );
    assert_eq!(
        repository
            .find_active_project_work_context_for_project(&created_context.project_id, 100)
            .unwrap(),
        Some(created_context.clone())
    );
    assert_eq!(
        repository
            .find_active_project_work_context_for_project(&created_context.project_id, 120)
            .unwrap(),
        None
    );

    let updated_context = ProjectWorkContext::new(
        created_context.id.clone(),
        created_context.surface,
        created_context.window_id.clone(),
        ProjectId::new("project-2"),
        240,
        created_context.created_at,
        40,
    );

    assert_eq!(
        repository
            .update_project_work_context(updated_context.clone())
            .unwrap(),
        updated_context.clone()
    );
    assert_eq!(
        repository
            .find_active_project_work_context_for_project(&ProjectId::new("project-2"), 200)
            .unwrap(),
        Some(updated_context.clone())
    );
    assert_eq!(
        repository
            .delete_expired_project_work_contexts(200)
            .unwrap(),
        0
    );
    assert_eq!(
        repository
            .delete_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
            .unwrap(),
        true
    );
    assert_eq!(
        repository
            .find_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
            .unwrap(),
        None
    );
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
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let task_repository = SqliteTaskRepository::new(pool.clone());
    let repository = SqliteSessionRepository::new(pool);
    project_repository
        .create_project(Project::new(
            ProjectId::new("project-1"),
            "Ora",
            "/tmp/ora",
            AuditFields::new(10, 10, false),
        ))
        .unwrap();
    task_repository
        .create_task(Task::new(
            TaskId::new("task-1"),
            ProjectId::new("project-1"),
            "Test sessions",
            TaskStatus::Todo,
            None,
            AuditFields::new(11, 11, false),
        ))
        .unwrap();
    let created_session = Session::new(
        SessionId::new("session-1"),
        TaskId::new("task-1"),
        AgentCli::OpenCode,
        "provider-1",
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
        created_session.agent_cli,
        created_session.agent_session_id.clone(),
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

/// Verifies a completed ACP handshake cannot attach a new session to a deleted task.
#[test]
fn session_repository_rejects_soft_deleted_task() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let cascade = SqliteCascadeRepository::new(pool.clone());
    assert_eq!(
        cascade.delete_task(&TaskId::new("task-1"), 20).unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    let session = Session::new(
        SessionId::new("session-after-delete"),
        TaskId::new("task-1"),
        AgentCli::OpenCode,
        "provider-after-delete",
        SessionStatus::Running,
        AuditFields::new(21, 21, false),
    );

    assert!(
        SqliteSessionRepository::new(pool)
            .create_session(session)
            .is_err()
    );
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
        AgentCli::OpenCode,
        "provider-1",
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

/// Verifies task aggregate deletion rejects running sessions and then commits every soft delete.
#[test]
fn task_cascade_delete_is_atomic_and_does_not_require_git() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Running);
    let repository = SqliteCascadeRepository::new(pool.clone());

    assert_eq!(
        repository.delete_task(&TaskId::new("task-1"), 20).unwrap(),
        CascadeDeleteOutcome::ActiveSession
    );
    assert_eq!(cascade_flags(&pool), (0, 0, 0, 0, 1));
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE sessions SET status = ?1 WHERE id = 'session-1'",
            rusqlite::params![SessionStatus::Stopped.database_value()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository.delete_task(&TaskId::new("task-1"), 30).unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    assert_eq!(cascade_flags(&pool), (0, 1, 1, 1, 1));
}

/// Verifies project deletion removes its transient lease and soft-deletes the full Ora aggregate.
#[test]
fn project_cascade_delete_removes_work_context_without_touching_external_state() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let repository = SqliteCascadeRepository::new(pool.clone());

    assert_eq!(
        repository
            .delete_project(&ProjectId::new("project-1"), 30)
            .unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    assert_eq!(cascade_flags(&pool), (1, 1, 1, 1, 0));
}

/// Inserts one complete aggregate using only Ora-owned rows, deliberately without Git fixtures.
fn insert_cascade_fixture(pool: &RepositoryPool, session_status: SessionStatus) {
    pool.with_connection(|connection| {
        connection.execute_batch(
            "INSERT INTO projects VALUES ('project-1', 'Ora', '/not/a/repository', 1, 1, 0);
             INSERT INTO tasks VALUES ('task-1', 'project-1', 'Task', 0, 'worktree-1', 1, 1, 0);
             INSERT INTO worktrees VALUES ('worktree-1', 'task-1', 'ora/task-1', 1, 1, 1, 0);
             INSERT INTO project_work_contexts VALUES ('context-1', 'web', 'main', 'project-1', 100, 1, 1);",
        )?;
        connection.execute(
            "INSERT INTO sessions VALUES ('session-1', 'task-1', ?1, 'provider-1', ?2, 1, 1, 0)",
            rusqlite::params![
                AgentCli::OpenCode.database_value(),
                session_status.database_value()
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

/// Reads all aggregate deletion markers plus the remaining transient work-context count.
fn cascade_flags(pool: &RepositoryPool) -> (i64, i64, i64, i64, i64) {
    pool.with_connection(|connection| {
        Ok((
            connection.query_row(
                "SELECT is_deleted FROM projects WHERE id = 'project-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row(
                "SELECT is_deleted FROM tasks WHERE id = 'task-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row(
                "SELECT is_deleted FROM worktrees WHERE id = 'worktree-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row(
                "SELECT is_deleted FROM sessions WHERE id = 'session-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row("SELECT COUNT(*) FROM project_work_contexts", [], |row| {
                row.get(0)
            })?,
        ))
    })
    .unwrap()
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
    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource {
            now: 1_700_000_000_000,
        })
        .bootstrap_repository_pool(
            &DatabaseLocation::path(database_path(&temp_dir)),
            &default_migration_catalog().unwrap(),
        )
        .unwrap()
    });

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
            "INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "session-invalid",
                "task-1",
                AgentCli::OpenCode.database_value(),
                "provider-invalid",
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
