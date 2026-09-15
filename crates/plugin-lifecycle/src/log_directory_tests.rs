//! Tests for the host-managed plugin log tree across uninstall: it follows the data disposition,
//! shares the staging transaction, and an open log handle fails the uninstall instead of
//! leaving a half-deleted plugin behind.

use crate::tests::{open_without_runtime, trace_logging_guard, write_plugin_package};
use crate::{PluginLifecycleError, PluginLogDirectories};
use ora_contracts::{PluginDataDisposition, UninstallPluginRequest};
use ora_domain::PluginId;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Writes a log for the example plugin the way a previous generation would have left it.
fn seed_log(data_directory: &Path) -> PathBuf {
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
    let directory = PluginLogDirectories::new(data_directory).path_for(&plugin_id);
    fs::create_dir_all(&directory).expect("create log directory");
    fs::write(directory.join("plugin.log"), "{\"a\":1}\n").expect("write log");
    directory
}

/// Deleting data removes the log tree and its emptied namespace directory; retaining data keeps
/// the log exactly as it was.
#[tokio::test]
async fn uninstall_disposition_decides_whether_the_log_tree_survives() {
    let _logging = trace_logging_guard();
    for (disposition, expect_log) in [
        (PluginDataDisposition::Delete, false),
        (PluginDataDisposition::Retain, true),
    ] {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "ora.example");
        let log_directory = seed_log(temp_dir.path());
        let lifecycle = open_without_runtime(temp_dir.path());

        lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "official/ora.example".to_string(),
                data_disposition: disposition,
            })
            .await
            .expect("uninstall");

        assert_eq!(
            (
                log_directory.join("plugin.log").is_file(),
                log_directory
                    .parent()
                    .expect("namespace directory")
                    .exists(),
            ),
            (expect_log, expect_log),
            "{disposition:?}"
        );
    }
}

/// On Windows an open handle on the active log keeps its directory from being renamed. The
/// uninstall must then report failure and roll the package and data back rather than report
/// success with the log directory still in place and possibly still being written.
#[cfg(windows)]
#[tokio::test]
async fn delete_uninstall_fails_and_rolls_back_while_the_log_is_open() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let log_directory = seed_log(temp_dir.path());
    let data_directory = temp_dir
        .path()
        .join("plugins")
        .join("data")
        .join("official")
        .join("ora.example");
    fs::create_dir_all(&data_directory).expect("create data directory");
    fs::write(data_directory.join("store.json"), "{}").expect("write data");
    let package_root = crate::tests::example_package_root(temp_dir.path());
    let lifecycle = open_without_runtime(temp_dir.path());
    let held_open = fs::File::open(log_directory.join("plugin.log")).expect("hold log open");

    let error = lifecycle
        .uninstall_plugin(UninstallPluginRequest {
            plugin_id: "official/ora.example".to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .err()
        .expect("uninstall reports the blocked log directory");
    drop(held_open);

    assert_eq!(
        (
            matches!(error, PluginLifecycleError::UninstallStaging { .. }),
            package_root.join("main.js").is_file(),
            data_directory.join("store.json").is_file(),
            log_directory.join("plugin.log").is_file(),
            lifecycle.list_installed_plugins().plugins.len(),
        ),
        (true, true, true, true, 1),
        "{error}"
    );
}
