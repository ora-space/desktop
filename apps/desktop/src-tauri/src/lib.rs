mod commands;
mod config;
mod error;
mod state;

use crate::config::DesktopConfigStore;
use crate::error::DesktopBootstrapError;
use crate::state::{DesktopRuntimeGuard, DesktopState};
use ora_backend::{Backend, BackendPaths};
use ora_logging::{
    FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy, init_logging,
    register_gitlancer_logger,
};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Starts the Tauri application with the persisted shared Backend and command adapters.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let (state, guard) = bootstrap_desktop(app)?;
            app.manage(state);
            app.manage(guard);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_project,
            commands::get_project,
            commands::list_projects,
            commands::update_project,
            commands::delete_project,
            commands::create_task,
            commands::get_task,
            commands::list_tasks,
            commands::update_task,
            commands::delete_task,
            commands::create_session,
            commands::get_session,
            commands::list_sessions,
            commands::respond_to_session_permission,
            commands::stop_session,
            commands::delete_session,
            commands::stream_contract,
            commands::cancel_contract_stream,
            commands::create_skill,
            commands::get_skill,
            commands::list_skills,
            commands::update_skill,
            commands::delete_skill,
            commands::create_agent,
            commands::get_agent,
            commands::list_agents,
            commands::update_agent,
            commands::delete_agent,
            commands::get_desktop_config,
            commands::set_worktree_root,
            commands::resolve_task_cwd,
            commands::open_location,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Resolves Desktop paths and constructs configuration, logging, and Backend state.
fn bootstrap_desktop(
    app: &mut tauri::App,
) -> Result<(DesktopState, DesktopRuntimeGuard), DesktopBootstrapError> {
    let app_data_directory = app
        .path()
        .app_data_dir()
        .map_err(DesktopBootstrapError::AppDataDirectory)?;
    let config = DesktopConfigStore::load_or_create(&app_data_directory)?;
    let config_snapshot = config.snapshot()?;
    let logging = init_logging(desktop_logging_config(&app_data_directory))?;
    register_gitlancer_logger();
    let backend = Backend::open(BackendPaths {
        database_path: app_data_directory.join("ora.sqlite3"),
        worktree_root: config_snapshot.worktree_root().to_path_buf(),
        home_directory: app
            .path()
            .home_dir()
            .map_err(DesktopBootstrapError::AppDataDirectory)?,
    })?;

    Ok((
        DesktopState {
            backend,
            config,
            stream_cancellations: Arc::new(Mutex::new(HashMap::new())),
        },
        DesktopRuntimeGuard { _logging: logging },
    ))
}

/// Builds the Desktop logging topology rooted in the stable system application directory.
fn desktop_logging_config(app_data_directory: &std::path::Path) -> LoggingConfig {
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

    LoggingConfig::new(LogLevel::Info, output)
}
