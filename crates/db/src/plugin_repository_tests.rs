use ora_application::PluginStateRepository;
use ora_domain::{PluginEnabledState, PluginId, PluginState};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, SqlitePluginStateRepository, TimestampSource,
    default_migration_catalog,
};

/// Supplies a deterministic migration timestamp without mutating process-wide clock state.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource {
    now: i64,
}

impl TimestampSource for FixedTimestampSource {
    /// Returns the fixed timestamp selected by the repository test fixture.
    fn current_timestamp_millis(&self) -> i64 {
        self.now
    }
}

/// Verifies the first enable persists a complete state that can be loaded through the port.
#[test]
fn persists_first_enabled_plugin_state() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin repository directory");
        let pool = DatabaseBootstrapper::new(FixedTimestampSource { now: 10 })
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("plugin-state.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin state database");
        let repository = SqlitePluginStateRepository::new(pool);
        let plugin_id = PluginId::new("ora.example");

        repository
            .set_plugin_enabled(&plugin_id, PluginEnabledState::Enabled, 20)
            .expect("enable plugin");

        assert_eq!(
            repository
                .find_plugin_state(&plugin_id)
                .expect("load plugin state"),
            Some(PluginState::new(
                plugin_id,
                PluginEnabledState::Enabled,
                20,
                20,
            )),
        );
    });
}

/// Verifies disabling an existing plugin preserves creation time and advances modification time.
#[test]
fn updates_existing_plugin_eligibility() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin repository directory");
        let pool = DatabaseBootstrapper::new(FixedTimestampSource { now: 10 })
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("plugin-state.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin state database");
        let repository = SqlitePluginStateRepository::new(pool);
        let plugin_id = PluginId::new("ora.example");
        repository
            .set_plugin_enabled(&plugin_id, PluginEnabledState::Enabled, 20)
            .expect("enable plugin");

        let updated = repository
            .set_plugin_enabled(&plugin_id, PluginEnabledState::Disabled, 30)
            .expect("disable plugin");
        let loaded = repository
            .find_plugin_state(&plugin_id)
            .expect("load disabled plugin state");
        let expected = PluginState::new(plugin_id, PluginEnabledState::Disabled, 20, 30);

        assert_eq!((updated, loaded), (expected.clone(), Some(expected)));
    });
}

/// Verifies reconciliation can load every durable row in deterministic identifier order.
#[test]
fn lists_plugin_states_in_identifier_order() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin repository directory");
        let pool = DatabaseBootstrapper::new(FixedTimestampSource { now: 10 })
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("plugin-state.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin state database");
        let repository = SqlitePluginStateRepository::new(pool);
        repository
            .set_plugin_enabled(&PluginId::new("ora.zeta"), PluginEnabledState::Enabled, 20)
            .expect("enable zeta plugin");
        repository
            .set_plugin_enabled(
                &PluginId::new("ora.alpha"),
                PluginEnabledState::Disabled,
                30,
            )
            .expect("disable alpha plugin");

        assert_eq!(
            repository.list_plugin_states().expect("list plugin states"),
            vec![
                PluginState::new(
                    PluginId::new("ora.alpha"),
                    PluginEnabledState::Disabled,
                    30,
                    30,
                ),
                PluginState::new(
                    PluginId::new("ora.zeta"),
                    PluginEnabledState::Enabled,
                    20,
                    20,
                ),
            ],
        );
    });
}

/// Verifies uninstall and reconciliation can physically remove one durable plugin-state row.
#[test]
fn deletes_plugin_state_by_identifier() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin repository directory");
        let pool = DatabaseBootstrapper::new(FixedTimestampSource { now: 10 })
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("plugin-state.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin state database");
        let repository = SqlitePluginStateRepository::new(pool);
        let plugin_id = PluginId::new("ora.example");
        repository
            .set_plugin_enabled(&plugin_id, PluginEnabledState::Enabled, 20)
            .expect("enable plugin");

        let deleted = repository
            .delete_plugin_state(&plugin_id)
            .expect("delete plugin state");
        let loaded = repository
            .find_plugin_state(&plugin_id)
            .expect("load deleted plugin state");

        assert_eq!((deleted, loaded), (true, None));
    });
}
