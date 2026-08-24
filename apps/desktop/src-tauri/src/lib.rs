mod commands;
mod config;
mod dashboard;
mod error;
mod open_external;
mod open_location;
mod settings_commands;
mod spec_commands;
mod state;
mod stream_forwarding;
mod surface;
mod workspace_files;

use crate::config::DesktopConfigStore;
use crate::error::DesktopBootstrapError;
use crate::state::{BundledBinaryPaths, DesktopRuntimeGuard, DesktopState};
use ora_backend::{Backend, BackendError, BackendPaths};
use ora_logging::{
    FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy, init_logging, ora_error,
    ora_info, ora_warn, register_gitlancer_logger,
};
use ora_runtime_settings::RuntimeLogLevelManager;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Expands the shared command registry (`app_commands.rs`) into the Tauri invoke handler.
///
/// The build script reads the same file to produce the ACL manifest, so a command cannot be
/// registered here without also entering ACL enforcement.
macro_rules! desktop_command_registry {
    ($($command:path),* $(,)?) => {
        tauri::generate_handler![$($command),*]
    };
}

const LOG_LEVEL_ENV_VAR: &str = "ORA_LOG_LEVEL";

/// The directory name under the user home where plugin and registry data lives.
const PLUGIN_HOME_DIRECTORY_NAME: &str = ".ora";

/// Starts the Tauri application with the persisted shared Backend and command adapters.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = surface::register_workbench_protocol(tauri::Builder::default());
    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let (state, guard) = bootstrap_desktop(app)?;
            surface::install(app.handle(), &state.surfaces, &state.backend);
            ora_info!(
                message = "bundled binary paths registered",
                ripgrep_path = %state.binary_paths.ripgrep_path().display(),
                deno_path = %state.binary_paths.deno_path().display(),
            );
            app.manage(state);
            app.manage(guard);
            Ok(())
        })
        .invoke_handler(include!("app_commands.rs"))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Resolves Desktop paths and constructs configuration, logging, and Backend state.
fn bootstrap_desktop(
    app: &mut tauri::App,
) -> Result<(DesktopState, DesktopRuntimeGuard), DesktopBootstrapError> {
    let app_data_directory = desktop_data_directory(app)?;
    let home_directory = app
        .path()
        .home_dir()
        .map_err(DesktopBootstrapError::AppDataDirectory)?;
    let config = DesktopConfigStore::load_or_create(&app_data_directory, &home_directory)?;
    let config_snapshot = config.snapshot()?;
    let resolved_timezone = read_system_timezone();
    let startup_override = read_desktop_log_level_override(|key| std::env::var(key).ok())?;
    let provisional_log_level = startup_override.unwrap_or(LogLevel::Info);
    let logging = init_logging(desktop_logging_config(
        &app_data_directory,
        resolved_timezone.timezone,
        provisional_log_level,
    ))?;
    let (logging_guard, level_control) = logging.into_parts();
    match &resolved_timezone.warning {
        Some(DesktopTimezoneWarning::SystemRead { error }) => {
            ora_warn!(
                message = "failed to read the system timezone, falling back to UTC",
                source = "system_timezone",
                error = %error,
                fallback_timezone = %resolved_timezone.timezone,
            );
        }
        Some(DesktopTimezoneWarning::InvalidTimezone { timezone }) => {
            ora_warn!(
                message = "invalid IANA system timezone, falling back to UTC",
                source = "system_timezone",
                timezone,
                fallback_timezone = %resolved_timezone.timezone,
            );
        }
        None => {}
    }
    register_gitlancer_logger();
    let binary_paths = match BundledBinaryPaths::resolve() {
        Ok(paths) => paths,
        Err(error) => {
            ora_error!(
                message = "required bundled binary is unavailable; stopping Desktop startup",
                error = %error,
            );
            return Err(error.into());
        }
    };
    let ripgrep_path = binary_paths.ripgrep_path().to_path_buf();
    let backend = Backend::open(BackendPaths {
        database_path: app_data_directory.join("ora.sqlite3"),
        data_directory: home_directory.join(PLUGIN_HOME_DIRECTORY_NAME),
        deno_path: binary_paths.deno_path().to_path_buf(),
        worktree_root: config_snapshot.worktree_root().to_path_buf(),
        home_directory,
        relative_path_base: desktop_relative_path_base(&app_data_directory),
        sessions_root: app_data_directory.join("sessions"),
        skills_root: app_data_directory.join("atoms").join("skills"),
        ripgrep_path: ripgrep_path.clone(),
        timezone: resolved_timezone.timezone,
    })?;
    let (configured_log_level, resolved_log_level) =
        tauri::async_runtime::block_on(load_desktop_log_level(&backend, startup_override))
            .map_err(DesktopBootstrapError::RuntimePreference)?;
    if resolved_log_level.effective_level != provisional_log_level {
        level_control.set_level(resolved_log_level.effective_level)?;
    }
    ora_info!(
        message = "logging initialized",
        timezone = %resolved_timezone.timezone,
        timezone_source = "system_timezone",
        log_level = %resolved_log_level.effective_level,
        log_level_source = resolved_log_level.source.as_str(),
    );
    let workspace_files = Arc::new(workspace_files::WorkspaceFileApi::new(ripgrep_path));
    let surfaces = surface::SurfaceService::new(app.handle().clone(), backend.plugin_gateway());
    let runtime_log_level = RuntimeLogLevelManager::new(
        level_control,
        backend.preferred_log_level_store(),
        configured_log_level,
        resolved_log_level.startup_override,
    );
    Ok((
        DesktopState {
            backend,
            config,
            runtime_log_level,
            workspace_files,
            binary_paths,
            app_data_directory: app_data_directory.clone(),
            stream_cancellations: Arc::new(Mutex::new(HashMap::new())),
            surfaces,
        },
        DesktopRuntimeGuard {
            _logging: logging_guard,
        },
    ))
}

/// Resolves the configured Desktop data root or falls back to Tauri's application data directory.
fn desktop_data_directory(app: &tauri::App) -> Result<std::path::PathBuf, DesktopBootstrapError> {
    if let Some(configured) = std::env::var_os("ORA_DATA_DIR") {
        let configured = std::path::PathBuf::from(configured);
        if configured.is_absolute() {
            return Ok(configured);
        }

        return Ok(std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(configured));
    }

    app.path()
        .app_data_dir()
        .map_err(DesktopBootstrapError::AppDataDirectory)
}

/// Resolves relative project roots against a stable directory, not process cwd.
///
/// `task run:desktop` points `ORA_DATA_DIR` at the repo `.data` directory shared
/// with the Desktop development environment. Project roots in that database are stored relative to
/// the repo root (the data directory's parent). Tauri starts in `src-tauri`, so
/// joining against live `current_dir()` would miss those roots.
fn desktop_relative_path_base(app_data_directory: &Path) -> PathBuf {
    if std::env::var_os("ORA_DATA_DIR").is_some() {
        return app_data_directory
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| app_data_directory.to_path_buf());
    }
    std::env::current_dir().unwrap_or_else(|_| app_data_directory.to_path_buf())
}

/// Carries the startup timezone selected from the operating system and any deferred warning.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedDesktopTimezone {
    timezone: chrono_tz::Tz,
    warning: Option<DesktopTimezoneWarning>,
}

/// Describes a recoverable Desktop system-timezone failure.
#[derive(Clone, Debug, Eq, PartialEq)]
enum DesktopTimezoneWarning {
    SystemRead { error: String },
    InvalidTimezone { timezone: String },
}

/// Carries the typed startup level and whether Desktop obtained it from the environment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolvedDesktopLogLevel {
    effective_level: LogLevel,
    startup_override: Option<LogLevel>,
    source: DesktopLogLevelSource,
}

/// Identifies the Desktop startup source without relying on an ambiguous boolean.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DesktopLogLevelSource {
    Environment,
    Preference,
}

impl DesktopLogLevelSource {
    /// Returns the stable source label recorded in the logging bootstrap event.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Preference => "preference",
        }
    }
}

/// Reads Desktop's optional startup override through an injected reader for deterministic tests.
fn read_desktop_log_level_override(
    mut read_variable: impl FnMut(&str) -> Option<String>,
) -> Result<Option<LogLevel>, DesktopBootstrapError> {
    let Some(raw_level) = read_variable(LOG_LEVEL_ENV_VAR) else {
        return Ok(None);
    };
    let level =
        raw_level
            .parse::<LogLevel>()
            .map_err(|error| DesktopBootstrapError::InvalidLogLevel {
                value: error.value().to_string(),
            })?;

    Ok(Some(level))
}

/// Resolves the runtime-ready Desktop level after Backend loads the shared preference.
fn resolve_desktop_log_level(
    startup_override: Option<LogLevel>,
    configured_level: LogLevel,
) -> ResolvedDesktopLogLevel {
    ResolvedDesktopLogLevel {
        effective_level: startup_override.unwrap_or(configured_level),
        startup_override,
        source: if startup_override.is_some() {
            DesktopLogLevelSource::Environment
        } else {
            DesktopLogLevelSource::Preference
        },
    }
}

/// Loads the shared preference before resolving the process-scoped Desktop override.
async fn load_desktop_log_level(
    backend: &Backend,
    startup_override: Option<LogLevel>,
) -> Result<(LogLevel, ResolvedDesktopLogLevel), BackendError> {
    let configured_level = backend.preferred_log_level().await?;
    Ok((
        configured_level,
        resolve_desktop_log_level(startup_override, configured_level),
    ))
}

/// Reads the operating system's IANA timezone once for the Desktop process lifetime.
fn read_system_timezone() -> ResolvedDesktopTimezone {
    resolve_system_timezone(iana_time_zone::get_timezone().map_err(|error| error.to_string()))
}

/// Validates an injected system-timezone result so failure branches remain unit-testable.
fn resolve_system_timezone(system_timezone: Result<String, String>) -> ResolvedDesktopTimezone {
    match system_timezone {
        Ok(timezone_name) => {
            let timezone_name = timezone_name.trim().to_string();
            match timezone_name.parse::<chrono_tz::Tz>() {
                Ok(timezone) => ResolvedDesktopTimezone {
                    timezone,
                    warning: None,
                },
                Err(_) => ResolvedDesktopTimezone {
                    timezone: chrono_tz::UTC,
                    warning: Some(DesktopTimezoneWarning::InvalidTimezone {
                        timezone: timezone_name,
                    }),
                },
            }
        }
        Err(error) => ResolvedDesktopTimezone {
            timezone: chrono_tz::UTC,
            warning: Some(DesktopTimezoneWarning::SystemRead { error }),
        },
    }
}

/// Builds the Desktop logging topology rooted in the stable system application directory.
fn desktop_logging_config(
    app_data_directory: &std::path::Path,
    timezone: chrono_tz::Tz,
    level: LogLevel,
) -> LoggingConfig {
    let file = FileLoggingConfig::new(
        app_data_directory.join("logs").join("ora.log"),
        RotationPolicy::Daily,
        NonZeroUsize::new(3).unwrap_or(NonZeroUsize::MIN),
    );
    let output = if cfg!(debug_assertions) {
        LogOutput::StdoutAndFile(file)
    } else {
        LogOutput::File(file)
    };

    LoggingConfig::new(level, output, timezone)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use std::num::NonZeroUsize;

    use ora_backend::{Backend, BackendPaths};
    use ora_logging::{FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy};
    use tempfile::TempDir;

    use super::{
        DesktopLogLevelSource, DesktopTimezoneWarning, ResolvedDesktopLogLevel,
        ResolvedDesktopTimezone, desktop_logging_config, load_desktop_log_level,
        read_desktop_log_level_override, resolve_desktop_log_level, resolve_system_timezone,
    };

    /// Verifies Desktop accepts every supported environment-backed log level.
    #[test]
    fn resolves_supported_desktop_log_levels() {
        for (raw, expected) in [
            ("trace", LogLevel::Trace),
            (" DEBUG ", LogLevel::Debug),
            ("Info", LogLevel::Info),
            ("wArN", LogLevel::Warn),
            ("ERROR", LogLevel::Error),
        ] {
            assert_eq!(
                resolve_desktop_log_level(
                    read_desktop_log_level_override(|_| Some(raw.to_string())).unwrap(),
                    LogLevel::Info,
                ),
                ResolvedDesktopLogLevel {
                    effective_level: expected,
                    startup_override: Some(expected),
                    source: DesktopLogLevelSource::Environment,
                }
            );
        }
    }

    /// Verifies a missing environment value selects the documented info default.
    #[test]
    fn uses_persisted_desktop_log_level_without_environment_override() {
        assert_eq!(
            resolve_desktop_log_level(
                read_desktop_log_level_override(|_| None).unwrap(),
                LogLevel::Warn,
            ),
            ResolvedDesktopLogLevel {
                effective_level: LogLevel::Warn,
                startup_override: None,
                source: DesktopLogLevelSource::Preference,
            }
        );
    }

    /// Verifies an unsupported environment value fails with the Desktop bootstrap error.
    #[test]
    fn rejects_unsupported_desktop_log_level() {
        let error = read_desktop_log_level_override(|_| Some("verbose".to_string())).unwrap_err();

        assert!(matches!(
            error,
            super::DesktopBootstrapError::InvalidLogLevel { value } if value == "verbose"
        ));
    }

    /// Verifies injected startup values do not leak into a later independent resolution.
    #[test]
    fn keeps_desktop_log_level_resolution_process_scoped() {
        let explicit = resolve_desktop_log_level(
            read_desktop_log_level_override(|_| Some("trace".to_string())).unwrap(),
            LogLevel::Info,
        );
        let later_default = resolve_desktop_log_level(
            read_desktop_log_level_override(|_| None).unwrap(),
            LogLevel::Warn,
        );

        assert_eq!(
            (explicit, later_default),
            (
                ResolvedDesktopLogLevel {
                    effective_level: LogLevel::Trace,
                    startup_override: Some(LogLevel::Trace),
                    source: DesktopLogLevelSource::Environment,
                },
                ResolvedDesktopLogLevel {
                    effective_level: LogLevel::Warn,
                    startup_override: None,
                    source: DesktopLogLevelSource::Preference,
                },
            )
        );
    }

    /// Verifies Desktop restores the SQLite preference and still gives an override precedence.
    #[tokio::test]
    async fn loads_persisted_desktop_log_level_after_restart() {
        let temp_dir = TempDir::new().unwrap();
        let first = Backend::open(test_backend_paths(temp_dir.path())).unwrap();
        assert_eq!(
            load_desktop_log_level(&first, None).await.unwrap(),
            (
                LogLevel::Info,
                ResolvedDesktopLogLevel {
                    effective_level: LogLevel::Info,
                    startup_override: None,
                    source: DesktopLogLevelSource::Preference,
                },
            )
        );
        first.set_preferred_log_level(LogLevel::Warn).await.unwrap();
        drop(first);

        let restarted = Backend::open(test_backend_paths(temp_dir.path())).unwrap();
        assert_eq!(
            load_desktop_log_level(&restarted, Some(LogLevel::Trace))
                .await
                .unwrap(),
            (
                LogLevel::Warn,
                ResolvedDesktopLogLevel {
                    effective_level: LogLevel::Trace,
                    startup_override: Some(LogLevel::Trace),
                    source: DesktopLogLevelSource::Environment,
                },
            )
        );
        assert_eq!(
            (
                DesktopLogLevelSource::Environment.as_str(),
                DesktopLogLevelSource::Preference.as_str(),
            ),
            ("environment", "preference")
        );
    }

    /// Verifies malformed SQLite log-level text aborts Desktop preference resolution.
    #[tokio::test]
    async fn rejects_malformed_persisted_desktop_log_level() {
        let temp_dir = TempDir::new().unwrap();
        let backend = Backend::open(test_backend_paths(temp_dir.path())).unwrap();
        drop(backend);
        rusqlite::Connection::open(temp_dir.path().join("ora.sqlite3"))
            .unwrap()
            .execute(
                "INSERT INTO user_config(key, value) VALUES ('log_level', 'verbose')",
                [],
            )
            .unwrap();
        let reopened = Backend::open(test_backend_paths(temp_dir.path())).unwrap();

        assert!(load_desktop_log_level(&reopened, None).await.is_err());
    }

    /// Verifies the resolved level is preserved in Desktop's fixed output topology.
    #[test]
    fn builds_desktop_logging_config_with_the_resolved_level() {
        let app_data_directory = std::env::temp_dir().join("ora-data");
        let config = desktop_logging_config(
            &app_data_directory,
            chrono_tz::Asia::Shanghai,
            LogLevel::Trace,
        );
        let file = FileLoggingConfig::new(
            app_data_directory.join("logs").join("ora.log"),
            RotationPolicy::Daily,
            NonZeroUsize::new(3).unwrap(),
        );
        let output = if cfg!(debug_assertions) {
            LogOutput::StdoutAndFile(file)
        } else {
            LogOutput::File(file)
        };

        assert_eq!(
            config,
            LoggingConfig::new(LogLevel::Trace, output, chrono_tz::Asia::Shanghai)
        );
    }

    /// Verifies Desktop accepts and trims a system-provided IANA timezone.
    #[test]
    fn resolves_valid_system_timezone() {
        assert_eq!(
            resolve_system_timezone(Ok("  Europe/London  ".to_string())),
            ResolvedDesktopTimezone {
                timezone: chrono_tz::Europe::London,
                warning: None,
            }
        );
    }

    /// Verifies an invalid system timezone remains visible while Desktop safely selects UTC.
    #[test]
    fn falls_back_when_system_timezone_is_invalid() {
        assert_eq!(
            resolve_system_timezone(Ok("London".to_string())),
            ResolvedDesktopTimezone {
                timezone: chrono_tz::UTC,
                warning: Some(DesktopTimezoneWarning::InvalidTimezone {
                    timezone: "London".to_string(),
                }),
            }
        );
    }

    /// Verifies an operating-system lookup failure remains visible while Desktop safely selects UTC.
    #[test]
    fn falls_back_when_system_timezone_lookup_fails() {
        assert_eq!(
            resolve_system_timezone(Err("timezone unavailable".to_string())),
            ResolvedDesktopTimezone {
                timezone: chrono_tz::UTC,
                warning: Some(DesktopTimezoneWarning::SystemRead {
                    error: "timezone unavailable".to_string(),
                }),
            }
        );
    }

    /// Builds a complete Backend path set rooted in one isolated Desktop data directory.
    fn test_backend_paths(root: &std::path::Path) -> BackendPaths {
        BackendPaths {
            database_path: root.join("ora.sqlite3"),
            data_directory: root.to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: root.join("worktrees"),
            home_directory: root.to_path_buf(),
            relative_path_base: root.to_path_buf(),
            sessions_root: root.join("sessions"),
            skills_root: root.join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        }
    }
}
