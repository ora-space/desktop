mod commands;
mod config;
mod dashboard;
mod error;
mod open_location;
mod skill_marketplace;
mod spec_commands;
mod state;
mod stream_forwarding;
mod workspace_files;

use crate::config::DesktopConfigStore;
use crate::error::DesktopBootstrapError;
use crate::state::{BundledBinaryPaths, DesktopRuntimeGuard, DesktopState};
use ora_backend::{Backend, BackendPaths};
use ora_logging::{
    FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy, init_logging, ora_error,
    ora_info, ora_warn, register_gitlancer_logger,
};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Starts the Tauri application with the persisted shared Backend and command adapters.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let (state, guard) = bootstrap_desktop(app)?;
            ora_info!(
                message = "bundled binary paths registered",
                ripgrep_path = %state.binary_paths.ripgrep_path().display(),
                deno_path = %state.binary_paths.deno_path().display(),
            );
            app.manage(state);
            app.manage(guard);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // =============================================================================
            // project
            // =============================================================================
            commands::create_project,
            commands::get_project,
            commands::list_projects,
            commands::list_project_branches,
            commands::update_project,
            commands::delete_project,
            // =============================================================================
            // task
            // =============================================================================
            commands::create_task,
            commands::get_task,
            commands::list_tasks,
            commands::update_task,
            commands::delete_task,
            spec_commands::get_task_workspace,
            commands::get_task_diff,
            commands::commit_task_changes,
            commands::push_task_branch,
            commands::list_task_diff_comments,
            commands::create_task_diff_comment,
            commands::reply_task_diff_comment,
            commands::set_task_diff_comment_status,
            // =============================================================================
            // fileSystem
            // =============================================================================
            commands::list_workspace_directory,
            commands::read_workspace_file,
            commands::search_workspace,
            spec_commands::get_spec_catalog,
            spec_commands::read_spec,
            // =============================================================================
            // session
            // =============================================================================
            commands::warm_session,
            commands::set_session_config,
            commands::attach_session,
            commands::get_session,
            commands::list_sessions,
            commands::respond_to_session_permission,
            commands::stop_session,
            commands::switch_session_agent,
            commands::resume_session_history,
            commands::delete_session,
            commands::rename_session,
            commands::stream_contract,
            commands::cancel_contract_stream,
            // =============================================================================
            // agentRuntime
            // =============================================================================
            commands::get_agent_runtime_status,
            // =============================================================================
            // skill
            // =============================================================================
            commands::create_skill,
            commands::get_skill,
            commands::list_skills,
            commands::update_skill,
            commands::delete_skill,
            skill_marketplace::open_skill_marketplace,
            // =============================================================================
            // agent
            // =============================================================================
            commands::prepare_skill_import,
            commands::get_skill_import,
            commands::commit_skill_import,
            commands::cancel_skill_import,
            commands::create_agent,
            commands::get_agent,
            commands::list_agents,
            // =============================================================================
            // plugin
            // =============================================================================
            commands::list_installed_plugins,
            commands::scan_plugins,
            commands::enable_plugin,
            commands::disable_plugin,
            commands::activate_plugin,
            commands::stop_plugin,
            commands::uninstall_plugin,
            commands::update_agent,
            commands::delete_agent,
            commands::prepare_agent_import,
            commands::commit_agent_import,
            // =============================================================================
            // gitIdentity
            // =============================================================================
            commands::get_git_identity,
            // =============================================================================
            // workflow
            // =============================================================================
            commands::create_workflow,
            commands::get_workflow,
            commands::list_workflows,
            commands::update_workflow,
            commands::delete_workflow,
            commands::get_workflow_draft,
            commands::update_workflow_draft,
            commands::publish_workflow,
            commands::rollback_workflow,
            commands::activate_workflow,
            commands::list_workflow_versions,
            commands::get_workflow_version,
            commands::delete_workflow_snapshot,
            commands::get_workflow_snapshot,
            // =============================================================================
            // workflowRun
            // =============================================================================
            commands::create_workflow_run,
            commands::get_workflow_run,
            commands::list_workflow_runs,
            commands::list_workflow_runs_by_workflow,
            commands::list_workflow_node_runs,
            commands::delete_workflow_run,
            commands::start_workflow_run,
            commands::cancel_workflow_run,
            commands::restart_workflow_run,
            commands::update_workflow_run_input,
            // =============================================================================
            // desktop
            // =============================================================================
            commands::get_desktop_config,
            commands::set_worktree_root,
            commands::resolve_task_cwd,
            open_location::open_location,
            commands::write_workflow_export,
            dashboard::get_dashboard_url,
            dashboard::get_dashboard_compare_url,
        ])
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
    let logging = init_logging(desktop_logging_config(
        &app_data_directory,
        resolved_timezone.timezone,
    ))?;
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
    ora_info!(
        message = "logging initialized",
        timezone = %resolved_timezone.timezone,
        timezone_source = "system_timezone",
    );
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
        data_directory: app_data_directory.clone(),
        deno_path: binary_paths.deno_path().to_path_buf(),
        worktree_root: config_snapshot.worktree_root().to_path_buf(),
        home_directory,
        relative_path_base: desktop_relative_path_base(&app_data_directory),
        sessions_root: app_data_directory.join("sessions"),
        skills_root: app_data_directory.join("atoms").join("skills"),
        ripgrep_path: ripgrep_path.clone(),
        timezone: resolved_timezone.timezone,
    })?;
    let workspace_files = Arc::new(workspace_files::WorkspaceFileApi::new(ripgrep_path));
    Ok((
        DesktopState {
            backend,
            config,
            workspace_files,
            binary_paths,
            app_data_directory: app_data_directory.clone(),
            stream_cancellations: Arc::new(Mutex::new(HashMap::new())),
        },
        DesktopRuntimeGuard { _logging: logging },
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

    LoggingConfig::new(LogLevel::Info, output, timezone)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{DesktopTimezoneWarning, ResolvedDesktopTimezone, resolve_system_timezone};

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
}
