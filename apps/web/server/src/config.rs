use crate::error::WebBootstrapError;
use ora_logging::{FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy};
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

const DATABASE_PATH_ENV_VAR: &str = "ORA_DB_PATH";
const PROJECT_NAME_ENV_VAR: &str = "ORA_PROJECT_NAME";
const PROJECT_PATH_ENV_VAR: &str = "ORA_PROJECT_PATH";
const WORK_DIR_ENV_VAR: &str = "ORA_WORK_DIR";
const HOST_ENV_VAR: &str = "ORA_HOST";
const PORT_ENV_VAR: &str = "ORA_PORT";
const LOG_LEVEL_ENV_VAR: &str = "ORA_LOG_LEVEL";
const LOG_MODE_ENV_VAR: &str = "ORA_LOG_MODE";
const LOG_PATH_ENV_VAR: &str = "ORA_LOG_PATH";
const LOG_MAX_DAYS_ENV_VAR: &str = "ORA_LOG_MAX_DAYS";

const DEFAULT_DATABASE_PATH: &str = "./ora.sqlite3";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 32578;
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_LOG_MODE: &str = "stdout";
const DEFAULT_LOG_PATH: &str = "./ora.log";
const DEFAULT_LOG_MAX_DAYS: &str = "3";

/// Groups the runtime configuration required to bootstrap the web server process.
pub struct RuntimeConfig {
    database: DatabaseConfig,
    project: ProjectConfig,
    server: ServerConfig,
    logging: LoggingConfig,
}

impl RuntimeConfig {
    /// Loads the runtime configuration from the environment-backed server contract.
    pub fn from_env() -> Result<Self, WebBootstrapError> {
        Self::from_reader(|key| env::var(key).ok())
    }

    /// Returns the database configuration used by the runtime bootstrap.
    pub fn database(&self) -> &DatabaseConfig {
        &self.database
    }

    /// Returns the configured bootstrap project identity used during startup reconciliation.
    pub fn project(&self) -> &ProjectConfig {
        &self.project
    }

    /// Returns the server bind configuration used by the runtime.
    pub fn server(&self) -> &ServerConfig {
        &self.server
    }

    /// Returns the shared logging configuration used during process bootstrap.
    pub fn logging(&self) -> &LoggingConfig {
        &self.logging
    }

    /// Loads the runtime configuration from a caller-provided variable reader for testability.
    pub(crate) fn from_reader(
        mut read_variable: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, WebBootstrapError> {
        let database = DatabaseConfig::from_reader(&mut read_variable)?;

        Ok(Self {
            project: ProjectConfig::from_reader(&mut read_variable, &database)?,
            database,
            server: ServerConfig::from_reader(&mut read_variable)?,
            logging: read_logging_config(&mut read_variable)?,
        })
    }
}

/// Describes the file-backed SQLite database location used by the web runtime.
pub struct DatabaseConfig {
    path: PathBuf,
}

impl DatabaseConfig {
    /// Returns the configured SQLite database path.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Loads the database path from a caller-provided variable reader for testability.
    fn from_reader(
        mut read_variable: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, WebBootstrapError> {
        let raw_path = read_variable(DATABASE_PATH_ENV_VAR)
            .unwrap_or_else(|| DEFAULT_DATABASE_PATH.to_string());

        if raw_path.trim().is_empty() {
            return Err(WebBootstrapError::InvalidDatabasePathEmpty);
        }

        Ok(Self {
            path: PathBuf::from(raw_path),
        })
    }
}

/// Describes the bootstrap project identity that startup reconciles into persistent storage.
pub struct ProjectConfig {
    name: String,
    path: PathBuf,
    work_dir: PathBuf,
}

impl ProjectConfig {
    /// Returns the configured project name used for bootstrap reconciliation.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the configured project root path used for bootstrap reconciliation.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns the configured linked-worktree root used for task-owned worktree provisioning.
    pub fn work_dir(&self) -> &Path {
        self.work_dir.as_path()
    }

    /// Loads the bootstrap project identity from a caller-provided variable reader for testability.
    fn from_reader(
        mut read_variable: impl FnMut(&str) -> Option<String>,
        database_config: &DatabaseConfig,
    ) -> Result<Self, WebBootstrapError> {
        let work_dir = match read_variable(WORK_DIR_ENV_VAR) {
            Some(work_dir) => {
                if work_dir.trim().is_empty() {
                    return Err(WebBootstrapError::InvalidWorkDirEmpty);
                }

                PathBuf::from(work_dir)
            }
            None => default_work_dir(database_config.path()),
        };

        Ok(Self {
            name: read_required_non_empty_variable(
                &mut read_variable,
                PROJECT_NAME_ENV_VAR,
                WebBootstrapError::InvalidProjectNameEmpty,
            )?,
            path: PathBuf::from(read_required_non_empty_variable(
                &mut read_variable,
                PROJECT_PATH_ENV_VAR,
                WebBootstrapError::InvalidProjectPathEmpty,
            )?),
            work_dir,
        })
    }
}

/// Derives the default linked-worktree root from the configured SQLite database location.
fn default_work_dir(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("worktrees")
}

/// Describes the host and port that the HTTP server binds to.
pub struct ServerConfig {
    host: IpAddr,
    port: u16,
}

impl ServerConfig {
    /// Returns the bind host used by the HTTP listener.
    pub fn host(&self) -> IpAddr {
        self.host
    }

    /// Returns the bind port used by the HTTP listener.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Combines the configured host and port into the socket address consumed by Tokio.
    pub fn socket_address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }

    /// Loads the bind host and port from a caller-provided variable reader for testability.
    fn from_reader(
        mut read_variable: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, WebBootstrapError> {
        let raw_host = read_variable(HOST_ENV_VAR).unwrap_or_else(|| DEFAULT_HOST.to_string());
        let host = raw_host
            .parse::<IpAddr>()
            .map_err(|source| WebBootstrapError::InvalidHost {
                value: raw_host.clone(),
                source,
            })?;
        let raw_port = read_variable(PORT_ENV_VAR).unwrap_or_else(|| DEFAULT_PORT.to_string());
        let port = raw_port
            .parse::<u16>()
            .map_err(|source| WebBootstrapError::InvalidPort {
                value: raw_port.clone(),
                source,
            })?;

        Ok(Self { host, port })
    }
}

/// Loads the logging configuration from the environment contract defined for the web server bootstrap.
fn read_logging_config(
    mut read_variable: impl FnMut(&str) -> Option<String>,
) -> Result<LoggingConfig, WebBootstrapError> {
    let level = match read_variable(LOG_LEVEL_ENV_VAR)
        .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        value => {
            return Err(WebBootstrapError::InvalidLogLevel {
                value: value.to_string(),
            });
        }
    };
    let file_config = FileLoggingConfig::new(
        read_variable(LOG_PATH_ENV_VAR).unwrap_or_else(|| DEFAULT_LOG_PATH.to_string()),
        RotationPolicy::Daily,
        read_log_max_days(&mut read_variable)?,
    );
    let output = match read_variable(LOG_MODE_ENV_VAR)
        .unwrap_or_else(|| DEFAULT_LOG_MODE.to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "stdout" => LogOutput::Stdout,
        "file" => LogOutput::File(file_config),
        "stdout_and_file" => LogOutput::StdoutAndFile(file_config),
        value => {
            return Err(WebBootstrapError::InvalidLogMode {
                value: value.to_string(),
            });
        }
    };

    Ok(LoggingConfig::new(level, output))
}

/// Parses the configured retention window and rejects zero-day values explicitly.
fn read_log_max_days(
    mut read_variable: impl FnMut(&str) -> Option<String>,
) -> Result<NonZeroUsize, WebBootstrapError> {
    let raw_value =
        read_variable(LOG_MAX_DAYS_ENV_VAR).unwrap_or_else(|| DEFAULT_LOG_MAX_DAYS.to_string());
    let parsed_value =
        raw_value
            .parse::<usize>()
            .map_err(|source| WebBootstrapError::InvalidLogMaxDays {
                value: raw_value.clone(),
                source,
            })?;

    NonZeroUsize::new(parsed_value).ok_or(WebBootstrapError::InvalidLogMaxDaysZero)
}

/// Reads one required environment variable and rejects blank values before bootstrap proceeds.
fn read_required_non_empty_variable(
    mut read_variable: impl FnMut(&str) -> Option<String>,
    variable_name: &str,
    empty_error: WebBootstrapError,
) -> Result<String, WebBootstrapError> {
    let value = read_variable(variable_name).unwrap_or_default();

    if value.trim().is_empty() {
        return Err(empty_error);
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{
        DATABASE_PATH_ENV_VAR, DEFAULT_DATABASE_PATH, DEFAULT_HOST, DEFAULT_PORT, DatabaseConfig,
        HOST_ENV_VAR, PORT_ENV_VAR, PROJECT_NAME_ENV_VAR, PROJECT_PATH_ENV_VAR, ProjectConfig,
        RuntimeConfig, ServerConfig, WORK_DIR_ENV_VAR,
    };
    use crate::error::WebBootstrapError;
    use pretty_assertions::assert_eq;

    /// Verifies the database configuration defaults to the documented SQLite path.
    #[test]
    fn loads_default_database_configuration() {
        let config = DatabaseConfig::from_reader(|_| None).unwrap_or_else(|error| {
            panic!("expected default database configuration to load: {error}");
        });

        assert_eq!(
            config.path().to_string_lossy().to_string(),
            DEFAULT_DATABASE_PATH.to_string()
        );
    }

    /// Verifies empty database paths fail with a typed bootstrap error.
    #[test]
    fn rejects_empty_database_path_configuration() {
        let error = match DatabaseConfig::from_reader(|key| match key {
            DATABASE_PATH_ENV_VAR => Some("   ".to_string()),
            _ => None,
        }) {
            Ok(_) => panic!("expected empty database path configuration to fail"),
            Err(error) => error,
        };

        assert!(matches!(error, WebBootstrapError::InvalidDatabasePathEmpty));
    }

    /// Verifies bootstrap project configuration requires both a non-empty name and path.
    #[test]
    fn rejects_missing_project_configuration() {
        let database_config = DatabaseConfig::from_reader(|_| None).unwrap();
        let error = match ProjectConfig::from_reader(|_| None, &database_config) {
            Ok(_) => panic!("expected missing project configuration to fail"),
            Err(error) => error,
        };

        assert!(matches!(error, WebBootstrapError::InvalidProjectNameEmpty));
    }

    /// Verifies blank bootstrap project paths fail with a typed bootstrap error.
    #[test]
    fn rejects_empty_project_path_configuration() {
        let error = match ProjectConfig::from_reader(
            |key| match key {
                PROJECT_NAME_ENV_VAR => Some("Ora".to_string()),
                PROJECT_PATH_ENV_VAR => Some("   ".to_string()),
                WORK_DIR_ENV_VAR => Some("/tmp/ora-worktrees".to_string()),
                _ => None,
            },
            &DatabaseConfig::from_reader(|_| None).unwrap(),
        ) {
            Ok(_) => panic!("expected empty project path configuration to fail"),
            Err(error) => error,
        };

        assert!(matches!(error, WebBootstrapError::InvalidProjectPathEmpty));
    }

    /// Verifies bootstrap project configuration exposes the configured identity unchanged.
    #[test]
    fn loads_project_configuration() {
        let config = ProjectConfig::from_reader(
            |key| match key {
                PROJECT_NAME_ENV_VAR => Some("Ora".to_string()),
                PROJECT_PATH_ENV_VAR => Some("/tmp/ora".to_string()),
                WORK_DIR_ENV_VAR => Some("/tmp/ora-worktrees".to_string()),
                _ => None,
            },
            &DatabaseConfig::from_reader(|_| None).unwrap(),
        )
        .unwrap_or_else(|error| panic!("expected project configuration to load: {error}"));

        assert_eq!(config.name(), "Ora");
        assert_eq!(
            config.path().to_string_lossy().to_string(),
            "/tmp/ora".to_string()
        );
        assert_eq!(
            config.work_dir().to_string_lossy().to_string(),
            "/tmp/ora-worktrees".to_string()
        );
    }

    /// Verifies blank linked-worktree roots fail with a typed bootstrap error.
    #[test]
    fn rejects_empty_work_dir_configuration() {
        let error = match ProjectConfig::from_reader(
            |key| match key {
                PROJECT_NAME_ENV_VAR => Some("Ora".to_string()),
                PROJECT_PATH_ENV_VAR => Some("/tmp/ora".to_string()),
                WORK_DIR_ENV_VAR => Some("   ".to_string()),
                _ => None,
            },
            &DatabaseConfig::from_reader(|_| None).unwrap(),
        ) {
            Ok(_) => panic!("expected empty worktree root configuration to fail"),
            Err(error) => error,
        };

        assert!(matches!(error, WebBootstrapError::InvalidWorkDirEmpty));
    }

    /// Verifies the linked-worktree root defaults to a `worktrees` sibling of the SQLite database path.
    #[test]
    fn defaults_work_dir_next_to_database_path() {
        let database_config = DatabaseConfig::from_reader(|key| match key {
            DATABASE_PATH_ENV_VAR => Some("/tmp/state/ora.sqlite3".to_string()),
            _ => None,
        })
        .unwrap_or_else(|error| panic!("expected database configuration to load: {error}"));
        let config = ProjectConfig::from_reader(
            |key| match key {
                PROJECT_NAME_ENV_VAR => Some("Ora".to_string()),
                PROJECT_PATH_ENV_VAR => Some("/tmp/ora".to_string()),
                _ => None,
            },
            &database_config,
        )
        .unwrap_or_else(|error| panic!("expected project configuration to load: {error}"));

        // Normalize separators so tests are portable across platforms (Windows uses `\`).
        let actual = config.work_dir().to_string_lossy().to_string().replace('\\', "/");
        assert_eq!(actual, "/tmp/state/worktrees".to_string());
    }

    /// Verifies the server configuration defaults to the documented host and port.
    #[test]
    fn loads_default_server_configuration() {
        let config = ServerConfig::from_reader(|_| None).unwrap_or_else(|error| {
            panic!("expected default server configuration to load: {error}");
        });

        assert_eq!(config.host().to_string(), DEFAULT_HOST.to_string());
        assert_eq!(config.port(), DEFAULT_PORT);
    }

    /// Verifies invalid port values fail with a typed bootstrap error.
    #[test]
    fn rejects_invalid_port_configuration() {
        let error = match ServerConfig::from_reader(|key| match key {
            HOST_ENV_VAR => Some(DEFAULT_HOST.to_string()),
            PORT_ENV_VAR => Some("not-a-port".to_string()),
            _ => None,
        }) {
            Ok(_) => panic!("expected invalid port configuration to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WebBootstrapError::InvalidPort { value, .. } if value == "not-a-port"
        ));
    }

    /// Verifies the runtime configuration loads both the server and logging contracts together.
    #[test]
    fn loads_runtime_configuration() {
        let config = RuntimeConfig::from_reader(|key| match key {
            PROJECT_NAME_ENV_VAR => Some("Ora".to_string()),
            PROJECT_PATH_ENV_VAR => Some("/tmp/ora".to_string()),
            WORK_DIR_ENV_VAR => Some("/tmp/ora-worktrees".to_string()),
            _ => None,
        })
        .unwrap_or_else(|error| panic!("expected runtime configuration to load: {error}"));

        assert_eq!(
            config.database().path().to_string_lossy().to_string(),
            DEFAULT_DATABASE_PATH.to_string()
        );
        assert_eq!(config.project().name(), "Ora");
        assert_eq!(
            config.project().path().to_string_lossy().to_string(),
            "/tmp/ora".to_string()
        );
        assert_eq!(
            config.project().work_dir().to_string_lossy().to_string(),
            "/tmp/ora-worktrees".to_string()
        );
        assert_eq!(
            config.server().socket_address().to_string(),
            format!("{DEFAULT_HOST}:{DEFAULT_PORT}")
        );
    }
}
