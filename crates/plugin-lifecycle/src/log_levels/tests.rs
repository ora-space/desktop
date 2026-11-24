use super::{DEFAULT_PLUGIN_LOG_LEVEL, PluginLogLevelState, PluginLogLevels};
use ora_domain::PluginId;
use ora_logging::LogLevel;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

fn id(name: &str) -> PluginId {
    PluginId::new("official", name).expect("plugin id")
}

/// Unknown values retain readable preferences and prevent destructive replacement.
#[test]
fn unknown_level_preserves_valid_entries_and_blocks_overwrite() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().unwrap();
        let levels = PluginLogLevels::open(temp.path());
        levels.set(&id("a"), LogLevel::Debug).unwrap();
        let path = temp.path().join("plugins").join("log-levels.json");
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        raw["official/b"] = serde_json::json!("future-level");
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let reopened = PluginLogLevels::open(temp.path());
        assert_eq!(
            reopened.state(&id("a")),
            PluginLogLevelState {
                level: LogLevel::Debug,
                configured: true,
                read_only_path: Some(path.clone()),
            }
        );
        assert!(reopened.set(&id("c"), LogLevel::Warn).is_err());
        let replaced: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(replaced, raw);
    });
}

/// Unset plugins report the default; a set level persists across reopen, publishes to live
/// subscribers, and leaves other plugins untouched.
#[test]
fn levels_are_per_plugin_persisted_and_published_live() {
    let temp = TempDir::new().expect("temp dir");
    let levels = PluginLogLevels::open(temp.path());
    let a_live = levels.subscribe(&id("a"));
    let b_live = levels.subscribe(&id("b"));
    assert_eq!(
        (levels.state(&id("a")), *a_live.borrow()),
        (
            PluginLogLevelState {
                level: DEFAULT_PLUGIN_LOG_LEVEL,
                configured: false,
                read_only_path: None,
            },
            LogLevel::Info
        )
    );

    levels.set(&id("a"), LogLevel::Debug).expect("set a");

    let reopened = PluginLogLevels::open(temp.path());
    assert_eq!(
        (
            *a_live.borrow(),
            *b_live.borrow(),
            levels.state(&id("b")),
            reopened.state(&id("a")),
            reopened.state(&id("b")),
        ),
        (
            LogLevel::Debug,
            LogLevel::Info,
            PluginLogLevelState {
                level: LogLevel::Info,
                configured: false,
                read_only_path: None,
            },
            PluginLogLevelState {
                level: LogLevel::Debug,
                configured: true,
                read_only_path: None,
            },
            PluginLogLevelState {
                level: LogLevel::Info,
                configured: false,
                read_only_path: None,
            },
        )
    );
}

/// A failed persist reports an error and changes neither the effective nor the stored level.
#[test]
fn a_failed_persist_changes_nothing() {
    let temp = TempDir::new().expect("temp dir");
    let levels = PluginLogLevels::open(temp.path());
    levels.set(&id("a"), LogLevel::Warn).expect("initial set");
    let live = levels.subscribe(&id("a"));
    // Occupying the parent path with a file makes the atomic replace fail.
    let plugins_dir = temp.path().join("plugins");
    std::fs::remove_dir_all(&plugins_dir).expect("remove plugins dir");
    std::fs::write(&plugins_dir, "not a directory").expect("occupy path");

    let error = levels
        .set(&id("a"), LogLevel::Trace)
        .err()
        .expect("persist fails");

    assert_eq!(
        (
            match error {
                super::PluginLogLevelPersistError::Write { path, .. } => path,
                other => panic!("{other}"),
            },
            levels.state(&id("a")),
            *live.borrow(),
        ),
        (
            plugins_dir.join("log-levels.json"),
            PluginLogLevelState {
                level: LogLevel::Warn,
                configured: true,
                read_only_path: None,
            },
            LogLevel::Warn,
        )
    );
}

/// Clearing removes the entry, publishes the default, and is a no-op for unset plugins.
#[test]
fn clear_restores_the_default_and_tolerates_unset_plugins() {
    let temp = TempDir::new().expect("temp dir");
    let levels = PluginLogLevels::open(temp.path());
    let live = levels.subscribe(&id("a"));
    levels.set(&id("a"), LogLevel::Error).expect("set");
    levels.clear(&id("a")).expect("clear");
    levels.clear(&id("never-set")).expect("clear unset");
    assert_eq!(
        (
            *live.borrow(),
            PluginLogLevels::open(temp.path()).state(&id("a")),
        ),
        (
            LogLevel::Info,
            PluginLogLevelState {
                level: LogLevel::Info,
                configured: false,
                read_only_path: None,
            }
        )
    );
}

/// Each load failure rejects edits and preserves exact bytes; repairing and reopening unlocks it.
#[test]
fn malformed_and_invalid_identity_files_remain_untouched_until_repaired() {
    ora_logging::with_trace_logging(|| {
        for bytes in [
            b"{not-json".as_slice(),
            br#"{"invalid id":"debug","official/a":"debug"}"#,
            br#"{"official/a":"debug","official/b":"future"}"#,
        ] {
            let temp = TempDir::new().unwrap();
            let path = temp.path().join("plugins").join("log-levels.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            let levels = PluginLogLevels::open(temp.path());
            let live = levels.subscribe(&id("a"));
            let before = levels.state(&id("a"));
            assert_eq!(before.read_only_path, Some(path.clone()));
            assert!(matches!(
                levels.set(&id("a"), LogLevel::Trace),
                Err(super::PluginLogLevelPersistError::ReadOnly { .. })
            ));
            assert!(matches!(
                levels.set(&id("c"), LogLevel::Warn),
                Err(super::PluginLogLevelPersistError::ReadOnly { .. })
            ));
            if bytes == b"{not-json" {
                assert!(matches!(
                    levels.clear(&id("a")),
                    Err(super::PluginLogLevelPersistError::ReadOnly { .. })
                ));
            }
            assert_eq!(
                (
                    std::fs::read(&path).unwrap(),
                    levels.state(&id("a")),
                    *live.borrow()
                ),
                (bytes.to_vec(), before.clone(), before.level)
            );
            std::fs::write(&path, br#"{"official/a":"debug"}"#).unwrap();
            let repaired = PluginLogLevels::open(temp.path());
            assert_eq!(
                repaired.state(&id("a")),
                PluginLogLevelState {
                    level: LogLevel::Debug,
                    configured: true,
                    read_only_path: None
                }
            );
            repaired.set(&id("c"), LogLevel::Warn).unwrap();
            assert_eq!(
                PluginLogLevels::open(temp.path()).state(&id("c")),
                PluginLogLevelState {
                    level: LogLevel::Warn,
                    configured: true,
                    read_only_path: None
                }
            );
        }
    });
}

/// A real filesystem read error must not be mistaken for an absent settings file.
#[test]
fn unreadable_file_blocks_set_and_clear_without_changing_the_path() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("plugins").join("log-levels.json");
        std::fs::create_dir_all(&path).unwrap();
        let sentinel = path.join("original");
        std::fs::write(&sentinel, b"preserve these bytes").unwrap();
        let levels = PluginLogLevels::open(temp.path());
        assert_eq!(
            levels.state(&id("a")),
            PluginLogLevelState {
                level: LogLevel::Info,
                configured: false,
                read_only_path: Some(path.clone())
            }
        );
        assert!(matches!(
            levels.set(&id("a"), LogLevel::Debug),
            Err(super::PluginLogLevelPersistError::ReadOnly { .. })
        ));
        assert!(matches!(
            levels.clear(&id("a")),
            Err(super::PluginLogLevelPersistError::ReadOnly { .. })
        ));
        assert!(path.is_dir());
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve these bytes");
    });
}

/// An explicit deletion may remove its own unsupported entry while preserving all other raw JSON.
#[test]
fn clear_only_removes_its_identity_and_retains_unknown_json_verbatim() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("plugins").join("log-levels.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let future = r#"{ "version": 184467440737095516161234, "nested": [true, "\\u0041"] }"#;
        std::fs::write(
            &path,
            format!(r#"{{"official/a":"debug","official/future":{future},"bad id":"trace"}}"#),
        )
        .unwrap();
        let levels = PluginLogLevels::open(temp.path());
        let live = levels.subscribe(&id("a"));
        levels.clear(&id("a")).unwrap();
        let raw: super::RawLevels = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            raw.keys().cloned().collect::<Vec<_>>(),
            vec!["bad id".to_owned(), "official/future".to_owned()]
        );
        assert_eq!(raw["official/future"].get(), future);
        assert_eq!(*live.borrow(), LogLevel::Info);
        levels.clear(&id("future")).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            serde_json::json!({"bad id":"trace"})
        );
    });
}

/// Warnings identify the file and plugin without disclosing unsupported setting values.
#[test]
fn unsupported_level_logs_the_repair_path_and_identity() {
    let temp = TempDir::new().unwrap();
    let log_path = temp.path().join("captured.log");
    let logs = std::sync::Arc::new(std::fs::File::create(&log_path).unwrap());
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(logs.clone());
    let path = temp.path().join("plugins").join("log-levels.json");
    ora_logging::with_recorded_trace_logging(layer, || {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"official/unknown":"secret-future-value"}"#).unwrap();
        let _levels = PluginLogLevels::open(temp.path());
    });
    let output = std::fs::read_to_string(log_path).unwrap();
    assert!(output.contains("WARN"));
    assert!(output.contains(path.to_string_lossy().as_ref()));
    assert!(output.contains("official/unknown"));
    assert!(!output.contains("secret-future-value"));
}
