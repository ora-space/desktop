//! Tests for the per-plugin log level: defaults, persistence, live delivery to a launched
//! generation, and its fate across retain- and delete-data uninstalls.

use crate::tests::{
    FakeRuntime, NoopNotificationSink, NoopStatusPublisher, launched_runtime, open_without_runtime,
    trace_logging_guard, write_plugin_package,
};
use crate::{
    LaunchedRuntime, PluginLaunchRequest, PluginLifecycle, PluginLifecycleConfig,
    PluginLifecycleError, PluginLogLevelState, PluginLogSetup, PluginRuntimeFailure,
    PluginRuntimeLauncher,
};
use ora_contracts::{ActivatePluginRequest, PluginDataDisposition, UninstallPluginRequest};
use ora_logging::{LogLevel, with_trace_logging};
use pretty_assertions::assert_eq;
use std::future::Future;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

/// Unset plugins report the default, a set level is reported back and survives reopening the
/// lifecycle, other identities are unaffected, and an uninstalled identity cannot be configured.
#[test]
fn levels_default_to_info_persist_and_stay_per_identity() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "ora.example");
        write_plugin_package(temp_dir.path(), "ora.other");
        let lifecycle = open_without_runtime(temp_dir.path());

        let before = lifecycle
            .plugin_log_level("official/ora.example")
            .expect("read default");
        let set = lifecycle
            .set_plugin_log_level("official/ora.example", LogLevel::Debug)
            .expect("set level");
        let missing = lifecycle
            .set_plugin_log_level("official/ora.missing", LogLevel::Debug)
            .err()
            .expect("uninstalled identity is refused");
        let reopened = open_without_runtime(temp_dir.path());

        assert_eq!(
            (
                before,
                set,
                matches!(missing, PluginLifecycleError::PluginNotFound { .. }),
                reopened
                    .plugin_log_level("official/ora.example")
                    .expect("read persisted"),
                reopened
                    .plugin_log_level("official/ora.other")
                    .expect("read other"),
            ),
            (
                PluginLogLevelState {
                    level: LogLevel::Info,
                    configured: false,
                },
                PluginLogLevelState {
                    level: LogLevel::Debug,
                    configured: true,
                },
                true,
                PluginLogLevelState {
                    level: LogLevel::Debug,
                    configured: true,
                },
                PluginLogLevelState {
                    level: LogLevel::Info,
                    configured: false,
                },
            )
        );
    });
}

/// The export path is the active file under the reserved directory of an installed identity;
/// an uninstalled identity is refused so the desktop cannot copy from an arbitrary location.
#[test]
fn log_file_is_resolved_only_for_installed_identities() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "ora.example");
        let lifecycle = open_without_runtime(temp_dir.path());

        let file = lifecycle
            .plugin_log_file("official/ora.example")
            .expect("installed plugin");
        let missing = lifecycle.plugin_log_file("official/ora.missing").err();

        assert_eq!(
            (
                file,
                matches!(missing, Some(PluginLifecycleError::PluginNotFound { .. })),
            ),
            (
                temp_dir
                    .path()
                    .join("plugins")
                    .join("logs")
                    .join("official")
                    .join("ora.example")
                    .join("plugin.log"),
                true,
            )
        );
    });
}

/// Captures the log setup handed to a launch so a test can observe the reserved directory, the
/// generation, and the live level subscription.
#[derive(Clone)]
struct LogCapturingLauncher {
    setups: mpsc::UnboundedSender<(PathBuf, PathBuf, u64, watch::Receiver<LogLevel>)>,
}

impl PluginRuntimeLauncher for LogCapturingLauncher {
    type Runtime = FakeRuntime;

    fn launch(
        &self,
        _request: PluginLaunchRequest,
        log: PluginLogSetup,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let setups = self.setups.clone();
        async move {
            setups
                .send((log.root, log.directory, log.generation, log.level))
                .map_err(|_| PluginRuntimeFailure::new("setup observer closed"))?;
            Ok(launched_runtime(FakeRuntime))
        }
    }
}

/// A launch is bound to the plugin's slot in `plugins/logs`, its attempt number, and the current level; a later
/// setting change reaches the running generation through the subscription without a restart.
#[tokio::test]
async fn launch_receives_the_reserved_directory_and_a_live_level() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (setups_tx, mut setups) = mpsc::unbounded_channel();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        LogCapturingLauncher { setups: setups_tx },
        NoopStatusPublisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Warn)
        .expect("preset level");

    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    let (root, directory, generation, mut level) = setups.recv().await.expect("launch observed");
    let at_launch = *level.borrow_and_update();
    lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Trace)
        .expect("change level while running");
    let changed = level.has_changed().expect("subscription alive");

    assert_eq!(
        (
            root,
            directory,
            generation,
            at_launch,
            changed,
            *level.borrow()
        ),
        (
            temp_dir.path().join("plugins").join("logs"),
            temp_dir
                .path()
                .join("plugins")
                .join("logs")
                .join("official")
                .join("ora.example"),
            1,
            LogLevel::Warn,
            true,
            LogLevel::Trace,
        )
    );
}

/// Retaining data on uninstall keeps the level for a reinstall of the same identity; deleting
/// data clears it so the identity restarts at the default.
#[tokio::test]
async fn uninstall_disposition_decides_whether_the_level_survives() {
    let _logging = trace_logging_guard();
    for (disposition, expected_after) in [
        (
            PluginDataDisposition::Retain,
            PluginLogLevelState {
                level: LogLevel::Error,
                configured: true,
            },
        ),
        (
            PluginDataDisposition::Delete,
            PluginLogLevelState {
                level: LogLevel::Info,
                configured: false,
            },
        ),
    ] {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "ora.example");
        let lifecycle = open_without_runtime(temp_dir.path());
        lifecycle
            .set_plugin_log_level("official/ora.example", LogLevel::Error)
            .expect("set level");

        lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "official/ora.example".to_string(),
                data_disposition: disposition,
            })
            .await
            .expect("uninstall");
        write_plugin_package(temp_dir.path(), "ora.example");
        let reinstalled = open_without_runtime(temp_dir.path());

        assert_eq!(
            reinstalled
                .plugin_log_level("official/ora.example")
                .expect("read level"),
            expected_after,
            "{disposition:?}"
        );
    }
}

/// A delete-data uninstall whose level clear cannot be persisted reports failure instead of a
/// complete cleanup, while the in-memory level stays what the file still says.
#[tokio::test]
async fn delete_uninstall_reports_a_failed_level_clear() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let lifecycle = open_without_runtime(temp_dir.path());
    lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Error)
        .expect("set level");
    // Making the settings file a directory defeats the atomic replace without touching the
    // package or data directories the uninstall itself moves.
    let levels_file = temp_dir.path().join("plugins").join("log-levels.json");
    std::fs::remove_file(&levels_file).expect("remove settings file");
    std::fs::create_dir(&levels_file).expect("occupy settings path");

    let error = lifecycle
        .uninstall_plugin(UninstallPluginRequest {
            plugin_id: "official/ora.example".to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .err()
        .expect("uninstall reports the failed clear");

    assert_eq!(
        (
            matches!(error, PluginLifecycleError::LogLevelPersistence { .. }),
            lifecycle
                .plugin_log_level("official/ora.example")
                .expect("read level"),
        ),
        (
            true,
            PluginLogLevelState {
                level: LogLevel::Error,
                configured: true,
            },
        ),
        "{error}"
    );
}
