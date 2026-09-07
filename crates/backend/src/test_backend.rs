use crate::BackendPaths;
use std::path::{Path, PathBuf};

/// Builds Backend paths with independently selectable application-data and Ora-home roots.
pub(crate) fn backend_paths(app_data_directory: &Path, home_directory: &Path) -> BackendPaths {
    ora_logging::initialize_test_clock();
    BackendPaths {
        app_data_directory: app_data_directory.to_path_buf(),
        home_directory: home_directory.to_path_buf(),
        deno_path: PathBuf::from("deno"),
        relative_path_base: app_data_directory.to_path_buf(),
        timezone: chrono_tz::UTC,
    }
}
