use ora_domain::ProjectId;

/// Creates a shared domain identifier during startup so the desktop shell stays compiled against the canonical domain crate.
fn bootstrap_project_id() -> ProjectId {
    ProjectId::new("desktop-bootstrap")
}

/// Starts the Tauri application and wires in development-only logging.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let _ = bootstrap_project_id();
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
