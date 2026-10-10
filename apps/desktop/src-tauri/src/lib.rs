mod cli;
mod commands;
mod diagnostic_logs;
mod error;
mod instance;
mod marketplace_sync;
mod open_external;
mod open_location;
mod state;
mod stream_forwarding;
mod stream_registry;
mod surface;
mod update;
mod workspace_files;

use crate::error::DesktopBootstrapError;
use crate::state::{BundledBinaryPaths, DesktopRuntimeGuard, DesktopState};
use crate::update::DesktopUpdateMode;
use ora_backend::{Backend, BackendPaths, Settings};
use ora_logging::{
    FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy, init_logging, ora_error,
    ora_info, ora_warn, register_gitlancer_logger,
};
use ora_runtime_settings::RuntimeLogLevelManager;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// Expands the shared command registry (`app_commands.rs`) into the Tauri invoke handler.
///
/// The build script reads the same file to produce the ACL manifest, so a command cannot be
/// registered here without also entering ACL enforcement.
macro_rules! desktop_command_registry {
    ($($command:path),* $(,)?) => {
        tauri::generate_handler![$($command),*]
    };
}

/// Logs emitted before storage opens use the same explicit default as an unset preference.
const DEFAULT_DESKTOP_LOG_LEVEL: LogLevel = LogLevel::Info;

/// The directory name under the user home where Ora-owned plugins and worktrees live.
const ORA_HOME_DIRECTORY_NAME: &str = ".ora";

/// Starts the Tauri application with the persisted shared Backend and command adapters.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Help and version must not build a window or open the database. A second
    // `ora-desktop --version` used to boot the whole application and mark live
    // workflow runs as interrupted by a restart.
    match cli::classify_args(std::env::args_os()) {
        cli::CliAction::Help => {
            cli::print_help();
            std::process::exit(0);
        }
        cli::CliAction::Version => {
            cli::print_version();
            std::process::exit(0);
        }
        cli::CliAction::Run => {}
    }
    let builder = surface::register_workbench_protocol(tauri::Builder::default())
        // Reveal the main window only once its splash has painted, so the logo is
        // centered from the moment the interface opens instead of showing a blank
        // window while the shell (and backend) initialize.
        .on_page_load(|webview, _payload| {
            if webview.label() == "main" {
                let _ = webview.window().show();
            }
        });
    let run_result = builder
        // The plugin exits a second process during its own setup, before this
        // process reaches backend bootstrap. The callback runs in the process
        // that already owns the window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Defer the heavy bootstrap (Backend open, SQLite, migrations) to a
            // background thread so `setup` returns immediately. The WebView then
            // paints the inline splash right away instead of holding a blank
            // window while the database initializes. The shell is revealed only
            // once the backend is ready (`ora-app-ready`), so there is no white
            // gap before or after the logo either.
            let handle = app.handle().clone();
            std::thread::spawn(move || match bootstrap_desktop(&handle) {
                Ok(DesktopBootstrap::Ready(ready)) => {
                    let (state, guard) = *ready;
                    ora_info!(
                        message = "bundled binary paths registered",
                        ripgrep_path = %state.binary_paths.ripgrep_path().display(),
                        deno_path = %state.binary_paths.deno_path().display(),
                        reaper_path = %state.binary_paths.reaper_path().display(),
                    );
                    surface::install(&handle, &state.surfaces, &state.backend);
                    handle.manage(state);
                    handle.manage(guard);
                    let _ = handle.emit_to("main", "ora-app-ready", ());
                }
                Ok(DesktopBootstrap::AlreadyRunning) => {
                    // The database was not opened. Leave through Tauri so
                    // RunEvent::Exit still runs. AppHandle::exit falls back to
                    // process::exit when the runtime cannot accept the request,
                    // which is what keeps this process from staying up with no
                    // backend if the event loop is not ready yet.
                    handle.exit(0);
                }
                Err(error) => {
                    ora_error!(
                        message = "desktop bootstrap failed",
                        error = %error,
                    );
                    // Let the shell stop waiting even when startup failed.
                    let _ = handle.emit_to("main", "ora-app-ready", ());
                }
            });
            Ok(())
        })
        .invoke_handler(include!("app_commands.rs"))
        .build(tauri::generate_context!())
        .map(|app| {
            app.run(|handle, event| {
                if matches!(event, tauri::RunEvent::Exit)
                    && let Some(state) = handle.try_state::<DesktopState>()
                {
                    state.streams.shutdown();
                }
            })
        });
    // Tauri has released managed backend state at this point, so process owners already had an
    // opportunity to shut down gracefully. The reaper now forcefully clears any survivors.
    if let Err(error) = ora_process::shutdown_reaper() {
        ora_error!(
            message = "process reaper failed during Desktop shutdown",
            error = %error,
        );
    }
    run_result.expect("error while running tauri application");
}

/// Startup either owns the data directory or must leave because another process does.
///
/// `Ready` is boxed so the exit result is not the size of the whole desktop state.
/// `AlreadyRunning` carries nothing, and the state is built only on the path that
/// opens the backend.
enum DesktopBootstrap {
    /// This process holds the instance lock and may serve the window.
    Ready(Box<(DesktopState, DesktopRuntimeGuard)>),
    /// Another live process holds the lock. The database was not opened.
    AlreadyRunning,
}

/// Resolves Desktop paths and constructs configuration, logging, and Backend state.
///
/// [`DesktopBootstrap::AlreadyRunning`] has not opened the database. The caller
/// exits through Tauri instead of continuing into the boot sweep.
fn bootstrap_desktop(app: &tauri::AppHandle) -> Result<DesktopBootstrap, DesktopBootstrapError> {
    let app_data_directory = desktop_data_directory(app)?;
    // Take the instance lock before logging, plugins, or SQLite. The boot sweep
    // inside Backend::open assumes the previous process is dead; the lock is that
    // proof. A busy lock means a live owner, so this process must not open the
    // database or mark its runs interrupted.
    let instance_lock = match instance::acquire_desktop_instance(&app_data_directory) {
        Ok(instance::DesktopInstance::Acquired(lock)) => lock,
        Ok(instance::DesktopInstance::AlreadyRunning { path }) => {
            // A GUI-subsystem build has no stderr, so eprintln would vanish for a
            // terminal launch. This is the same stdout / parent-console path as
            // --version. Returning here must not open SQLite.
            cli::write_parent_console(&format!(
                "Ora Desktop is already running (lock {}); this process will exit without opening the database.",
                path.display()
            ));
            return Ok(DesktopBootstrap::AlreadyRunning);
        }
        Err(error) => return Err(DesktopBootstrapError::InstanceLock(error)),
    };
    let user_home_directory = app
        .path()
        .home_dir()
        .map_err(DesktopBootstrapError::OraHomeDirectory)?;
    let home_directory = user_home_directory.join(ORA_HOME_DIRECTORY_NAME);
    let resolved_timezone = read_system_timezone();
    let logging = initialize_desktop_logging(&app_data_directory, resolved_timezone.timezone)?;
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
    ora_process::initialize_reaper(binary_paths.reaper_path())
        .map_err(DesktopBootstrapError::ProcessReaper)?;
    let backend_paths = BackendPaths {
        app_data_directory: app_data_directory.clone(),
        home_directory: home_directory.clone(),
        deno_path: binary_paths.deno_path().to_path_buf(),
        relative_path_base: desktop_relative_path_base(&app_data_directory),
        timezone: resolved_timezone.timezone,
    };
    let home_directory = backend_paths.home_directory.clone();
    let backend = Backend::open(backend_paths)?;
    let configured_log_level = tauri::async_runtime::block_on(restore_desktop_log_level(
        backend.settings(),
        &level_control,
    ))?;
    ora_info!(
        message = "logging initialized",
        timezone = %resolved_timezone.timezone,
        timezone_source = "system_timezone",
        log_level = %configured_log_level,
    );
    let workspace_files = Arc::new(workspace_files::WorkspaceFileApi::new(
        binary_paths.ripgrep_path().to_path_buf(),
    ));
    let surfaces = surface::SurfaceService::new(app.clone(), backend.plugins().gateway());
    let update = update::UpdateService::start(
        app.clone(),
        backend.settings().clone(),
        &home_directory,
        resolved_timezone.timezone,
        if cfg!(debug_assertions) {
            DesktopUpdateMode::Disabled
        } else {
            DesktopUpdateMode::Enabled
        },
    )
    .map_err(DesktopBootstrapError::Update)?;
    // Unlike release updates, a marketplace refresh only rebuilds a cached listing, so it runs in
    // development builds too rather than staying untested until a packaged release.
    let marketplace_sync = marketplace_sync::MarketplaceSyncService::start(
        app.clone(),
        backend.plugins(),
        resolved_timezone.timezone,
    )
    .map_err(DesktopBootstrapError::MarketplaceSync)?;
    let runtime_log_level = RuntimeLogLevelManager::new(
        level_control,
        backend.settings().preferred_log_level_store(),
        configured_log_level,
    );
    Ok(DesktopBootstrap::Ready(Box::new((
        DesktopState {
            backend,
            update,
            runtime_log_level,
            workspace_files,
            binary_paths,
            streams: stream_registry::StreamRegistry::default(),
            surfaces,
        },
        DesktopRuntimeGuard {
            _logging: logging_guard,
            _marketplace_sync: marketplace_sync,
            _instance_lock: instance_lock,
        },
    ))))
}

/// Resolves the configured Desktop data root or falls back to Tauri's application data directory.
fn desktop_data_directory(
    app: &tauri::AppHandle,
) -> Result<std::path::PathBuf, DesktopBootstrapError> {
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

/// Resolves relative local Workspace locations against a stable directory, not process cwd.
///
/// `task run:desktop` points `ORA_DATA_DIR` at the repo `.data` directory shared
/// with the Desktop development environment. Workspace locations in that database are stored relative to
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

/// Installs Desktop's real process logger before storage is opened, with an explicit default.
fn initialize_desktop_logging(
    app_data_directory: &Path,
    timezone: chrono_tz::Tz,
) -> Result<ora_logging::InitializedLogging, ora_logging::LoggingInitError> {
    init_logging(desktop_logging_config(
        app_data_directory,
        timezone,
        DEFAULT_DESKTOP_LOG_LEVEL,
    ))
}

/// Restores storage before startup proceeds; read failures must not become default preferences.
async fn restore_desktop_log_level(
    settings: &Settings,
    control: &ora_logging::LogLevelControl,
) -> Result<LogLevel, DesktopBootstrapError> {
    let level = settings
        .preferred_log_level()
        .await
        .map_err(DesktopBootstrapError::RuntimePreference)?;
    control.set_level(level)?;
    Ok(level)
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
        DEFAULT_DESKTOP_LOG_LEVEL, DesktopTimezoneWarning, ResolvedDesktopTimezone,
        desktop_logging_config, initialize_desktop_logging, resolve_system_timezone,
        restore_desktop_log_level,
    };

    /// Runs the real startup logger in fresh processes for both first launch and restart.
    #[test]
    fn legacy_log_level_environment_does_not_affect_startup() {
        for value in ["trace", " DEBUG ", "error", "verbose", ""] {
            let directory = TempDir::new().unwrap();
            for expected in ["info", "warn"] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "tests::desktop_logging_startup_child",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("ORA_LOG_LEVEL", value)
                    .env("ORA_TEST_LOGGING_DIRECTORY", directory.path())
                    .env("ORA_TEST_EXPECTED_LOG_LEVEL", expected)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "startup failed for legacy value {value:?}, expected {expected}:\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
                assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            }
        }
    }

    /// Exercises production initialization without sharing its process-global clock or subscriber.
    #[test]
    #[ignore = "run only by the parent test in an isolated subprocess"]
    fn desktop_logging_startup_child() {
        let directory =
            std::path::PathBuf::from(std::env::var_os("ORA_TEST_LOGGING_DIRECTORY").unwrap());
        let expected = std::env::var("ORA_TEST_EXPECTED_LOG_LEVEL")
            .unwrap()
            .parse::<LogLevel>()
            .unwrap();
        // This child runs only this test. Its real global subscriber must remain active so
        // neither a test clock nor a scoped test dispatcher bypasses production initialization.
        let logging = initialize_desktop_logging(&directory, chrono_tz::Asia::Shanghai).unwrap();
        let (_guard, control) = logging.into_parts();
        assert_eq!(control.current_level().unwrap(), LogLevel::Info);

        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let backend = Backend::open(test_backend_paths(&directory)).unwrap();
            let restored = restore_desktop_log_level(backend.settings(), &control)
                .await
                .unwrap();
            assert_eq!(
                (restored, control.current_level().unwrap()),
                (expected, expected)
            );
            let manager = ora_runtime_settings::RuntimeLogLevelManager::new(
                control.clone(),
                backend.settings().preferred_log_level_store(),
                restored,
            );
            assert_eq!(
                manager.set_level(LogLevel::Warn).await.unwrap(),
                ora_runtime_settings::RuntimeLogLevelState {
                    configured_level: LogLevel::Warn,
                    effective_level: LogLevel::Warn,
                },
            );
            assert_eq!(control.current_level().unwrap(), LogLevel::Warn);
        });
    }

    /// Distinguishes invalid preferences and real storage failures from an unset preference.
    #[test]
    fn rejects_unreadable_persisted_desktop_log_level() {
        ora_logging::with_trace_logging(|| {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                for statement in [
                    "INSERT INTO user_config(key, value) VALUES ('log_level', 'verbose')",
                    "DROP TABLE user_config",
                ] {
                    let temp_dir = TempDir::new().unwrap();
                    let backend = Backend::open(test_backend_paths(temp_dir.path())).unwrap();
                    rusqlite::Connection::open(temp_dir.path().join("ora.sqlite3"))
                        .unwrap()
                        .execute(statement, [])
                        .unwrap();
                    let control = ora_logging::test_log_level_control(DEFAULT_DESKTOP_LOG_LEVEL);
                    assert!(matches!(
                        restore_desktop_log_level(backend.settings(), &control).await,
                        Err(super::DesktopBootstrapError::RuntimePreference(_))
                    ));
                    assert_eq!(control.current_level().unwrap(), LogLevel::Info);
                }
            })
        });
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
        ora_logging::initialize_test_clock();
        BackendPaths {
            app_data_directory: root.to_path_buf(),
            home_directory: root.to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            relative_path_base: root.to_path_buf(),
            timezone: chrono_tz::UTC,
        }
    }
}
