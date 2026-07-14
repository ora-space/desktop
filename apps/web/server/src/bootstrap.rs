use crate::app_state::AppState;
use crate::config::{ProjectConfig, RuntimeConfig};
use crate::error::WebBootstrapError;
use crate::service::{ProjectApi, ProjectWorkContextApi, SessionApi, TaskApi};
use ora_application::{
    Clock, OpenProjectWorkContextHandler, ProjectIdGenerator, ProjectRepository,
    ProjectRepositoryError, UuidProjectIdGenerator, UuidProjectWorkContextIdGenerator,
};
use ora_contracts::{OpenProjectWorkContextRequest, ProjectWorkContextSurface};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use ora_domain::{AuditFields, Project};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Builds the application state used by the web runtime from SQLite-backed dependencies.
pub fn build_app_state(runtime_config: &RuntimeConfig) -> Result<AppState, WebBootstrapError> {
    let pool = build_repository_pool(runtime_config.database().path())?;
    let clock = SystemClock;

    reconcile_configured_project(&pool, runtime_config.project(), clock)?;

    Ok(AppState::new(
        Arc::new(ProjectApi::new(pool.clone(), clock)),
        Arc::new(ProjectWorkContextApi::new(pool.clone(), clock)),
        Arc::new(TaskApi::new(
            pool.clone(),
            runtime_config.project().path().to_path_buf(),
            runtime_config.project().work_dir().to_path_buf(),
            clock,
        )),
        Arc::new(SessionApi::new(pool, clock)),
    ))
}

/// Builds application state for tests that need SQLite-backed handlers without project reconciliation.
#[cfg(test)]
pub(crate) fn build_app_state_for_database(
    database_path: &Path,
    project_root: &Path,
    work_dir: &Path,
) -> Result<AppState, WebBootstrapError> {
    let pool = build_repository_pool(database_path)?;
    let clock = SystemClock;

    Ok(AppState::new(
        Arc::new(ProjectApi::new(pool.clone(), clock)),
        Arc::new(ProjectWorkContextApi::new(pool.clone(), clock)),
        Arc::new(TaskApi::new(
            pool.clone(),
            project_root.to_path_buf(),
            work_dir.to_path_buf(),
            clock,
        )),
        Arc::new(SessionApi::new(pool, clock)),
    ))
}

/// Ensures the configured workspace project exists in persistent storage before readiness.
fn reconcile_configured_project(
    pool: &RepositoryPool,
    project_config: &ProjectConfig,
    clock: SystemClock,
) -> Result<(), WebBootstrapError> {
    let repository = ora_db::SqliteProjectRepository::new(pool.clone());
    let context_repository = ora_db::SqliteProjectWorkContextRepository::new(pool.clone());
    let configured_project_path = project_config.path().to_string_lossy().to_string();
    let existing_project = repository
        .find_project_by_name(project_config.name())
        .map_err(project_bootstrap_error)?;

    let project_id = match existing_project {
        Some(existing_project) if existing_project.root_path == configured_project_path => {
            Ok(existing_project.id)
        }
        Some(existing_project) => {
            let updated_at = clock.now_timestamp_millis();

            repository
                .update_project(Project::new(
                    existing_project.id,
                    existing_project.name,
                    configured_project_path,
                    AuditFields::new(
                        existing_project.audit_fields.created_at,
                        updated_at,
                        existing_project.audit_fields.is_deleted,
                    ),
                ))
                .map(|project| project.id)
                .map_err(project_bootstrap_error)
        }
        None => {
            let now = clock.now_timestamp_millis();

            repository
                .create_project(Project::new(
                    UuidProjectIdGenerator::new().generate_project_id(),
                    project_config.name(),
                    configured_project_path,
                    AuditFields::new(now, now, false),
                ))
                .map(|project| project.id)
                .map_err(project_bootstrap_error)
        }
    }?;
    let handler = OpenProjectWorkContextHandler::new(
        repository,
        context_repository,
        UuidProjectWorkContextIdGenerator::new(),
        clock,
    );

    handler
        .handle(OpenProjectWorkContextRequest {
            surface: ProjectWorkContextSurface::Web,
            window_id: "main".to_string(),
            project_id: project_id.to_string(),
        })
        .map(|_| ())
        .map_err(project_work_context_bootstrap_error)
}

/// Opens the configured file-backed SQLite database and returns the shared repository pool.
fn build_repository_pool(database_path: &Path) -> Result<RepositoryPool, WebBootstrapError> {
    let catalog = default_migration_catalog().map_err(WebBootstrapError::DatabaseBootstrap)?;
    let database_parent = database_path.parent().unwrap_or_else(|| Path::new("."));

    fs::create_dir_all(database_parent).map_err(WebBootstrapError::DataDirectoryCreate)?;

    DatabaseBootstrapper::system()
        .bootstrap_repository_pool(&DatabaseLocation::path(database_path), &catalog)
        .map_err(WebBootstrapError::DatabaseBootstrap)
}

/// Converts repository-owned bootstrap failures into one stable startup error variant.
fn project_bootstrap_error(error: ProjectRepositoryError) -> WebBootstrapError {
    match error {
        ProjectRepositoryError::OperationFailed(message) => {
            WebBootstrapError::ProjectBootstrap { message }
        }
    }
}

/// Converts project work context bootstrap failures into one stable startup error variant.
fn project_work_context_bootstrap_error(
    error: ora_application::ApplicationError,
) -> WebBootstrapError {
    WebBootstrapError::ProjectBootstrap {
        message: error.to_string(),
    }
}

/// Reads the current wall-clock time for audit fields in the runtime.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    /// Returns the current Unix timestamp in milliseconds for handler audit fields.
    fn now_timestamp_millis(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_millis() as i64,
            Err(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_app_state, build_app_state_for_database};
    use crate::config::RuntimeConfig;
    use crate::error::WebBootstrapError;
    use ora_application::{ProjectRepository, ProjectWorkContextRepository};
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository,
        SqliteProjectWorkContextRepository, default_migration_catalog,
    };
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Verifies bootstrap fails cleanly when the configured database path points to a directory.
    #[test]
    fn rejects_directory_database_paths() {
        let temp_dir = TempDir::new().unwrap();
        let error = match build_app_state_for_database(
            temp_dir.path(),
            temp_dir.path(),
            &temp_dir.path().join("worktrees"),
        ) {
            Ok(_) => panic!("expected directory database path to fail"),
            Err(error) => error,
        };

        assert!(matches!(error, WebBootstrapError::DatabaseBootstrap(_)));
    }

    /// Verifies runtime bootstrap creates the configured project when no visible row exists yet.
    #[test]
    fn creates_configured_project_during_bootstrap() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join("bootstrap-create");
        let project_path = temp_dir.path().join("workspace").join("ora");
        let runtime_config = runtime_config(&data_dir, "Ora", &project_path);
        let database_path = data_dir.join("ora.sqlite3");

        build_app_state(&runtime_config)
            .unwrap_or_else(|error| panic!("expected runtime bootstrap to succeed: {error}"));

        let repository = bootstrapped_project_repository(&database_path);
        let context_repository = bootstrapped_project_work_context_repository(&database_path);

        assert_eq!(
            repository
                .find_project_by_name("Ora")
                .unwrap()
                .map(|project| (
                    project.name,
                    project.root_path,
                    project.audit_fields.is_deleted,
                )),
            Some((
                "Ora".to_string(),
                project_path.to_string_lossy().to_string(),
                false,
            ))
        );
        let project = repository
            .find_project_by_name("Ora")
            .unwrap()
            .unwrap_or_else(|| panic!("expected configured project to exist after bootstrap"));

        assert_eq!(
            context_repository
                .find_project_work_context(ora_domain::ProjectWorkContextSurface::Web, "main")
                .unwrap()
                .map(|context| (context.surface, context.window_id, context.project_id)),
            Some((
                ora_domain::ProjectWorkContextSurface::Web,
                "main".to_string(),
                project.id,
            ))
        );
    }

    /// Verifies runtime bootstrap leaves an already reconciled configured project unchanged.
    #[test]
    fn keeps_configured_project_unchanged_when_name_and_path_match() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join("bootstrap-noop");
        let project_path = temp_dir.path().join("workspace").join("ora");
        let runtime_config = runtime_config(&data_dir, "Ora", &project_path);
        let database_path = data_dir.join("ora.sqlite3");

        build_app_state(&runtime_config)
            .unwrap_or_else(|error| panic!("expected first runtime bootstrap to succeed: {error}"));
        let repository = bootstrapped_project_repository(&database_path);
        let original_project = repository
            .find_project_by_name("Ora")
            .unwrap()
            .unwrap_or_else(|| panic!("expected configured project to exist after bootstrap"));

        build_app_state(&runtime_config).unwrap_or_else(|error| {
            panic!("expected second runtime bootstrap to succeed: {error}")
        });
        let repository = bootstrapped_project_repository(&database_path);

        assert_eq!(
            repository.find_project_by_name("Ora").unwrap(),
            Some(original_project)
        );
    }

    /// Verifies runtime bootstrap repairs path drift without replacing the persisted project identity.
    #[test]
    fn updates_configured_project_path_when_storage_drifts() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join("bootstrap-update");
        let original_project_path = temp_dir.path().join("workspace").join("ora");
        let original_runtime_config = runtime_config(&data_dir, "Ora", &original_project_path);
        let database_path = data_dir.join("ora.sqlite3");

        build_app_state(&original_runtime_config)
            .unwrap_or_else(|error| panic!("expected first runtime bootstrap to succeed: {error}"));
        let repository = bootstrapped_project_repository(&database_path);
        let original_project = repository
            .find_project_by_name("Ora")
            .unwrap()
            .unwrap_or_else(|| panic!("expected configured project to exist after bootstrap"));

        thread::sleep(Duration::from_millis(2));

        let updated_project_path = temp_dir.path().join("workspace").join("ora-renamed");
        let updated_runtime_config = runtime_config(&data_dir, "Ora", &updated_project_path);
        build_app_state(&updated_runtime_config).unwrap_or_else(|error| {
            panic!("expected second runtime bootstrap to succeed: {error}")
        });
        let repository = bootstrapped_project_repository(&database_path);
        let updated_project = repository
            .find_project_by_name("Ora")
            .unwrap()
            .unwrap_or_else(|| panic!("expected configured project to exist after path update"));

        assert_eq!(updated_project.id, original_project.id);
        assert_eq!(updated_project.name, original_project.name);
        assert_eq!(
            updated_project.root_path,
            updated_project_path.to_string_lossy().to_string()
        );
        assert_eq!(
            updated_project.audit_fields.created_at,
            original_project.audit_fields.created_at
        );
        assert!(
            updated_project.audit_fields.updated_at >= original_project.audit_fields.updated_at
        );
    }

    /// Builds one runtime configuration without mutating process environment during tests.
    fn runtime_config(data_dir: &Path, project_name: &str, project_path: &Path) -> RuntimeConfig {
        RuntimeConfig::from_reader(|key| match key {
            "ORA_DATA_DIR" => Some(data_dir.to_string_lossy().to_string()),
            "ORA_PROJECT_NAME" => Some(project_name.to_string()),
            "ORA_PROJECT_PATH" => Some(project_path.to_string_lossy().to_string()),
            _ => None,
        })
        .unwrap_or_else(|error| panic!("expected runtime configuration to load: {error}"))
    }

    /// Opens the test database so bootstrap assertions can inspect persisted project state.
    fn bootstrapped_project_repository(database_path: &Path) -> SqliteProjectRepository {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().unwrap(),
            )
            .unwrap_or_else(|error| {
                panic!("expected repository pool bootstrap to succeed: {error}")
            });

        SqliteProjectRepository::new(pool)
    }

    /// Opens the test database so bootstrap assertions can inspect persisted project work context state.
    fn bootstrapped_project_work_context_repository(
        database_path: &Path,
    ) -> SqliteProjectWorkContextRepository {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().unwrap(),
            )
            .unwrap_or_else(|error| {
                panic!("expected repository pool bootstrap to succeed: {error}")
            });

        SqliteProjectWorkContextRepository::new(pool)
    }
}
