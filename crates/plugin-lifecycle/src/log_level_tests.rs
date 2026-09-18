//! Tests for the per-plugin log level: defaults, persistence, live delivery to a launched
//! generation, host-session identity, and its fate across retain- and delete-data uninstalls.

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
use ora_logging::LogLevel;
use pretty_assertions::assert_eq;
use std::future::Future;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

/// Unset plugins report the default, a set level is reported back and survives reopening the
/// lifecycle, other identities are unaffected, and an uninstalled identity cannot be configured.
#[tokio::test]
async fn levels_default_to_info_persist_and_stay_per_identity() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    write_plugin_package(temp_dir.path(), "ora.other");
    let lifecycle = open_without_runtime(temp_dir.path());

    let before = lifecycle
        .plugin_log_level("official/ora.example")
        .expect("read default");
    let set = lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Debug)
        .await
        .expect("set level");
    let missing = lifecycle
        .set_plugin_log_level("official/ora.missing", LogLevel::Debug)
        .await
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
}

/// The export path is the active file under the reserved directory of an installed identity;
/// an uninstalled identity is refused so the desktop cannot copy from an arbitrary location.
#[test]
fn log_file_is_resolved_only_for_installed_identities() {
    ora_logging::with_trace_logging(|| {
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

/// What a launch was told about its log: the roots, the host session, the generation, and the
/// live level subscription.
struct CapturedLogSetup {
    root: PathBuf,
    directory: PathBuf,
    host_session_id: String,
    generation: u64,
    level: watch::Receiver<LogLevel>,
}

/// Captures the log setup handed to a launch so a test can observe it.
#[derive(Clone)]
struct LogCapturingLauncher {
    setups: mpsc::UnboundedSender<CapturedLogSetup>,
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
                .send(CapturedLogSetup {
                    root: log.root,
                    directory: log.directory,
                    host_session_id: log.host_session_id,
                    generation: log.generation,
                    level: log.level,
                })
                .map_err(|_| PluginRuntimeFailure::new("setup observer closed"))?;
            Ok(launched_runtime(FakeRuntime))
        }
    }
}

/// Opens a lifecycle over `data_directory` whose launches report their log setup on the
/// returned channel.
fn open_capturing(
    data_directory: &std::path::Path,
) -> (
    PluginLifecycle<LogCapturingLauncher, NoopStatusPublisher, NoopNotificationSink>,
    mpsc::UnboundedReceiver<CapturedLogSetup>,
) {
    let (setups_tx, setups) = mpsc::unbounded_channel();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: data_directory.to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        LogCapturingLauncher { setups: setups_tx },
        NoopStatusPublisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    (lifecycle, setups)
}

/// A launch is bound to the plugin's slot in `plugins/logs`, its attempt number, the host
/// session, and the current level; a later setting change reaches the running generation
/// through the subscription without a restart.
#[tokio::test]
async fn launch_receives_the_reserved_directory_and_a_live_level() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (lifecycle, mut setups) = open_capturing(temp_dir.path());
    lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Warn)
        .await
        .expect("preset level");

    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    let mut setup = setups.recv().await.expect("launch observed");
    let at_launch = *setup.level.borrow_and_update();
    lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Trace)
        .await
        .expect("change level while running");
    let changed = setup.level.has_changed().expect("subscription alive");

    assert_eq!(
        (
            setup.root,
            setup.directory,
            setup.generation,
            setup.host_session_id.is_empty(),
            at_launch,
            changed,
            *setup.level.borrow()
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
            false,
            LogLevel::Warn,
            true,
            LogLevel::Trace,
        )
    );
}

/// Two host runs over the same data directory both launch the plugin as generation 1, and only
/// the session id they hand to the log tells those two generations apart.
#[tokio::test]
async fn each_host_run_mints_a_fresh_session_for_equal_generations() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let (lifecycle, mut setups) = open_capturing(temp_dir.path());
        lifecycle
            .activate_plugin(ActivatePluginRequest {
                plugin_id: "official/ora.example".to_string(),
            })
            .await
            .expect("activate plugin");
        let setup = setups.recv().await.expect("launch observed");
        sessions.push((setup.generation, setup.host_session_id));
    }

    assert_eq!(
        (
            sessions[0].0,
            sessions[1].0,
            sessions[0].1 == sessions[1].1,
            sessions[0].1.is_empty(),
        ),
        (1, 1, false, false)
    );
}

/// Retaining data on uninstall keeps the level for a reinstall of the same identity; deleting
/// data clears it so the identity restarts at the default, and a late update after that
/// delete is refused rather than recreating the setting.
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
            .await
            .expect("set level");

        lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "official/ora.example".to_string(),
                data_disposition: disposition,
            })
            .await
            .expect("uninstall");
        let late_update = lifecycle
            .set_plugin_log_level("official/ora.example", LogLevel::Trace)
            .await
            .err()
            .expect("an uninstalled identity cannot be configured");
        write_plugin_package(temp_dir.path(), "ora.example");
        let reinstalled = open_without_runtime(temp_dir.path());

        assert_eq!(
            (
                matches!(late_update, PluginLifecycleError::PluginNotFound { .. }),
                reinstalled
                    .plugin_log_level("official/ora.example")
                    .expect("read level"),
            ),
            (true, expected_after),
            "{disposition:?}"
        );
    }
}

/// A delete-data uninstall whose level clear cannot be persisted reports failure and rolls the
/// package, data tree, and log tree back to their exact paths: the plugin stays installed, the
/// setting stays what the file still says, and nothing is reported as cleaned up.
#[tokio::test]
async fn delete_uninstall_rolls_back_every_tree_when_the_level_clear_fails() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let package_root = crate::tests::example_package_root(temp_dir.path());
    let data_directory = temp_dir
        .path()
        .join("plugins")
        .join("data")
        .join("official")
        .join("ora.example");
    std::fs::create_dir_all(&data_directory).expect("create data directory");
    std::fs::write(data_directory.join("store.json"), "{}").expect("write data");
    let log_directory = temp_dir
        .path()
        .join("plugins")
        .join("logs")
        .join("official")
        .join("ora.example");
    std::fs::create_dir_all(&log_directory).expect("create log directory");
    std::fs::write(log_directory.join("plugin.log"), "{\"a\":1}\n").expect("write log");
    let lifecycle = open_without_runtime(temp_dir.path());
    lifecycle
        .set_plugin_log_level("official/ora.example", LogLevel::Error)
        .await
        .expect("set level");
    // Making the settings file a directory defeats the atomic replace without touching the
    // package, data, or log directories the uninstall itself moves.
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
            package_root.join("main.js").is_file(),
            data_directory.join("store.json").is_file(),
            log_directory.join("plugin.log").is_file(),
            lifecycle.list_installed_plugins().plugins.len(),
        ),
        (
            true,
            PluginLogLevelState {
                level: LogLevel::Error,
                configured: true,
            },
            true,
            true,
            true,
            1,
        ),
        "{error}"
    );
}
