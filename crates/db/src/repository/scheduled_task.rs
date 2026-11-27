use ora_application::{RepositoryError, ScheduledTaskRepository, ScheduledTaskUpdateResolution};
use ora_domain::{
    AgentRef, AuditFields, ProjectId, ScheduledTask, ScheduledTaskExecutionLocation,
    ScheduledTaskId, ScheduledTaskSchedule,
};
use rusqlite::{Row, params};

use crate::repository::scheduled_approval::{approval_columns, map_approval_columns};
use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists Scheduled Task definitions through the shared SQLite pool.
#[derive(Clone, Debug)]
pub struct SqliteScheduledTaskRepository {
    pool: RepositoryPool,
}

impl SqliteScheduledTaskRepository {
    /// Builds a scheduled-task repository from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl ScheduledTaskRepository for SqliteScheduledTaskRepository {
    fn create_scheduled_task(&self, task: ScheduledTask) -> Result<ScheduledTask, RepositoryError> {
        let schedule_json = serialize_schedule(&task.schedule)?;
        let revision = serialize_revision(task.revision)?;
        let (approval_policy, approval_consented_at) = approval_columns(task.approval.as_ref());
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO scheduled_tasks (
                        id, name, enabled, schedule_json, execution_location, project_id,
                        agent_ref, model, prompt, revision, approval_policy,
                        approval_consented_at, schedule_anchor_at, worktree_base_branch,
                        created_at, updated_at, is_deleted
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
                     )",
                    params![
                        task.id.as_ref(),
                        &task.name,
                        bool_to_sqlite(task.enabled),
                        schedule_json,
                        execution_location_column(task.execution_location),
                        task.project_id.as_ref(),
                        task.agent_ref.as_str(),
                        task.model.as_deref(),
                        &task.prompt,
                        revision,
                        approval_policy,
                        approval_consented_at,
                        task.schedule_anchor_at,
                        task.worktree_base_branch.as_deref(),
                        task.audit_fields.created_at,
                        task.audit_fields.updated_at,
                        bool_to_sqlite(task.audit_fields.is_deleted),
                    ],
                )?;
                Ok(task)
            })
            .map_err(scheduled_task_repository_error_from_database)
    }

    fn find_scheduled_task(
        &self,
        task_id: &ScheduledTaskId,
    ) -> Result<Option<ScheduledTask>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, name, enabled, schedule_json, execution_location, project_id,
                            agent_ref, model, prompt, revision, approval_policy,
                            approval_consented_at, schedule_anchor_at, worktree_base_branch,
                            created_at, updated_at, is_deleted
                     FROM scheduled_tasks
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![task_id.as_ref()])?;
                rows.next()?.map(map_scheduled_task_row).transpose()
            })
            .map_err(scheduled_task_repository_error_from_database)
    }

    fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTask>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, name, enabled, schedule_json, execution_location, project_id,
                            agent_ref, model, prompt, revision, approval_policy,
                            approval_consented_at, schedule_anchor_at, worktree_base_branch,
                            created_at, updated_at, is_deleted
                     FROM scheduled_tasks
                     WHERE is_deleted = 0
                     ORDER BY created_at, id",
                )?;
                let mut rows = statement.query([])?;
                let mut tasks = Vec::new();
                while let Some(row) = rows.next()? {
                    tasks.push(map_scheduled_task_row(row)?);
                }
                Ok(tasks)
            })
            .map_err(scheduled_task_repository_error_from_database)
    }

    fn update_scheduled_task(
        &self,
        task: ScheduledTask,
        expected_updated_at: i64,
    ) -> Result<ScheduledTaskUpdateResolution, RepositoryError> {
        let task_id = task.id.clone();
        let schedule_json = serialize_schedule(&task.schedule)?;
        let revision = serialize_revision(task.revision)?;
        let (approval_policy, approval_consented_at) = approval_columns(task.approval.as_ref());
        // The expected timestamp makes the replacement conditional so a concurrent edit,
        // revocation, or soft delete can never be silently overwritten by a stale writer.
        let updated = self
            .pool
            .with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE scheduled_tasks
                         SET name = ?2, enabled = ?3, schedule_json = ?4, project_id = ?5,
                             agent_ref = ?6, model = ?7, prompt = ?8, revision = ?9,
                             approval_policy = ?10, approval_consented_at = ?11,
                             schedule_anchor_at = ?12, worktree_base_branch = ?13,
                             updated_at = ?14
                         WHERE id = ?1 AND is_deleted = 0 AND updated_at = ?15",
                        params![
                            task.id.as_ref(),
                            &task.name,
                            bool_to_sqlite(task.enabled),
                            schedule_json,
                            task.project_id.as_ref(),
                            task.agent_ref.as_str(),
                            task.model.as_deref(),
                            &task.prompt,
                            revision,
                            approval_policy,
                            approval_consented_at,
                            task.schedule_anchor_at,
                            task.worktree_base_branch.as_deref(),
                            task.audit_fields.updated_at,
                            expected_updated_at,
                        ],
                    )
                    .map(|rows| rows > 0)
                    .map_err(Into::into)
            })
            .map_err(scheduled_task_repository_error_from_database)?;
        // `updated == false` means no row matched the expected snapshot: the definition is either
        // gone (NotFound) or concurrently moved on (Conflict).
        self.find_scheduled_task(&task_id)
            .map(|found| match (updated, found) {
                (true, Some(task)) => ScheduledTaskUpdateResolution::Updated(Box::new(task)),
                (false, Some(_)) => ScheduledTaskUpdateResolution::Conflict,
                (_, None) => ScheduledTaskUpdateResolution::NotFound,
            })
    }

    fn soft_delete_scheduled_task(
        &self,
        task_id: &ScheduledTaskId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE scheduled_tasks
                         SET enabled = 0, updated_at = ?2, is_deleted = 1
                         WHERE id = ?1 AND is_deleted = 0",
                        params![task_id.as_ref(), deleted_at],
                    )
                    .map(|rows| rows > 0)
                    .map_err(Into::into)
            })
            .map_err(scheduled_task_repository_error_from_database)
    }
}

fn map_scheduled_task_row(row: &Row<'_>) -> Result<ScheduledTask, crate::DatabaseError> {
    let schedule = deserialize_schedule(&row.get::<_, String>("schedule_json")?)?;
    let execution_location = match row.get::<_, String>("execution_location")?.as_str() {
        "local" => ScheduledTaskExecutionLocation::Local,
        "cloud" => ScheduledTaskExecutionLocation::Cloud,
        value => {
            return Err(crate::DatabaseError::CorruptScheduledTaskExecutionLocation(
                value.to_string(),
            ));
        }
    };
    let revision_value = row.get::<_, i64>("revision")?;
    let revision = u64::try_from(revision_value)
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or(crate::DatabaseError::CorruptScheduledTaskRevision(
            revision_value.to_string(),
        ))?;
    let approval = map_approval_columns(
        row.get::<_, Option<String>>("approval_policy")?,
        row.get::<_, Option<i64>>("approval_consented_at")?,
    )?;
    let mut task = ScheduledTask::new(
        ScheduledTaskId::new(row.get::<_, String>("id")?),
        row.get::<_, String>("name")?,
        row.get::<_, i64>("enabled")? != 0,
        schedule,
        execution_location,
        ProjectId::new(row.get::<_, String>("project_id")?),
        AgentRef::parse(row.get::<_, String>("agent_ref")?)?,
        row.get("model")?,
        row.get::<_, String>("prompt")?,
        approval,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    )?;
    task.revision = revision;
    task.schedule_anchor_at = row.get("schedule_anchor_at")?;
    task.worktree_base_branch = row.get("worktree_base_branch")?;
    Ok(task)
}

fn serialize_schedule(schedule: &ScheduledTaskSchedule) -> Result<String, RepositoryError> {
    serde_json::to_string(schedule).map_err(|error| {
        scheduled_task_repository_error_from_database(
            crate::DatabaseError::CorruptScheduledTaskSchedule(error),
        )
    })
}

fn serialize_revision(revision: u64) -> Result<i64, RepositoryError> {
    i64::try_from(revision).map_err(|_| {
        scheduled_task_repository_error_from_database(
            crate::DatabaseError::CorruptScheduledTaskRevision(revision.to_string()),
        )
    })
}

fn deserialize_schedule(value: &str) -> Result<ScheduledTaskSchedule, crate::DatabaseError> {
    let schedule: ScheduledTaskSchedule =
        serde_json::from_str(value).map_err(crate::DatabaseError::CorruptScheduledTaskSchedule)?;
    match schedule {
        ScheduledTaskSchedule::EveryHours { interval } => {
            ScheduledTaskSchedule::every_hours(interval).map_err(Into::into)
        }
        ScheduledTaskSchedule::Daily { hour, minute } => {
            ScheduledTaskSchedule::daily(hour, minute).map_err(Into::into)
        }
        ScheduledTaskSchedule::Weekly {
            weekdays,
            hour,
            minute,
        } => ScheduledTaskSchedule::weekly(weekdays, hour, minute).map_err(Into::into),
    }
}

fn execution_location_column(location: ScheduledTaskExecutionLocation) -> &'static str {
    match location {
        ScheduledTaskExecutionLocation::Local => "local",
        ScheduledTaskExecutionLocation::Cloud => "cloud",
    }
}

fn scheduled_task_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use ora_application::{
        ProjectRepository, ScheduledTaskRepository, ScheduledTaskUpdateResolution,
    };
    use ora_domain::{
        AgentRef, AuditFields, Project, ProjectId, ScheduledTask, ScheduledTaskApproval,
        ScheduledTaskExecutionLocation, ScheduledTaskId, ScheduledTaskSchedule,
        ScheduledTaskWeekday, WorkspaceLocation,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteProjectRepository,
        default_migration_catalog, test_clock::TestClock,
    };

    use super::SqliteScheduledTaskRepository;

    #[test]
    fn definitions_round_trip_update_and_soft_delete() {
        let (temp_dir, pool) = bootstrapped_pool();
        seed_project(&pool, temp_dir.path());
        let repository = SqliteScheduledTaskRepository::new(pool);
        let original = scheduled_task();

        let created = repository
            .create_scheduled_task(original.clone())
            .expect("create task");
        assert_eq!(created, original);
        assert_eq!(
            repository
                .find_scheduled_task(&original.id)
                .expect("find task"),
            Some(original.clone())
        );

        let mut replacement = original.clone();
        replacement.name = "Weekly quality report".to_string();
        replacement.schedule = ScheduledTaskSchedule::weekly(
            BTreeSet::from([ScheduledTaskWeekday::Monday, ScheduledTaskWeekday::Friday]),
            9,
            30,
        )
        .expect("weekly schedule");
        replacement.revision = 2;
        replacement.audit_fields.updated_at = 20;
        assert_eq!(
            repository
                .update_scheduled_task(replacement.clone(), /*expected_updated_at*/ 10)
                .expect("update task"),
            ScheduledTaskUpdateResolution::Updated(Box::new(replacement.clone()))
        );
        assert_eq!(
            repository.list_scheduled_tasks().expect("list tasks"),
            vec![replacement.clone()]
        );

        // A writer holding a stale snapshot must lose against the committed replacement.
        let mut stale = replacement.clone();
        stale.name = "Stale overwrite".to_string();
        assert_eq!(
            repository
                .update_scheduled_task(stale, /*expected_updated_at*/ 10)
                .expect("detect stale update"),
            ScheduledTaskUpdateResolution::Conflict
        );
        assert_eq!(
            repository.list_scheduled_tasks().expect("list tasks"),
            vec![replacement.clone()]
        );

        assert!(
            repository
                .soft_delete_scheduled_task(&replacement.id, 30)
                .expect("delete task")
        );
        assert_eq!(
            repository
                .find_scheduled_task(&replacement.id)
                .expect("find deleted task"),
            None
        );
        assert_eq!(
            repository
                .update_scheduled_task(replacement, /*expected_updated_at*/ 20)
                .expect("update deleted task"),
            ScheduledTaskUpdateResolution::NotFound
        );
        assert!(
            repository
                .list_scheduled_tasks()
                .expect("list deleted tasks")
                .is_empty()
        );
    }

    fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
        let temp_dir = TempDir::new().expect("create temp directory");
        let database_path = temp_dir.path().join("scheduled-task.sqlite3");
        let pool = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap database");
        (temp_dir, pool)
    }

    fn seed_project(pool: &RepositoryPool, temp_root: &std::path::Path) {
        let workspace_root = temp_root.join("project");
        std::fs::create_dir_all(&workspace_root).expect("create project workspace");
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
    }

    fn scheduled_task() -> ScheduledTask {
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
        .expect("task");
        task.reanchor_on_enable(10);
        task.freeze_worktree_base_branch("main");
        task
    }
}
