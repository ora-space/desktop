//! Covers Hook install outcomes after the host dropped plugin enablement.

use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::user_config::UserConfigApi;
use ora_contracts::{ImportPluginRequest, InstallOutcome, ListInstalledPluginsRequest};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// Opens a throwaway SQLite pool under `root` for PluginApi tests.
fn test_pool(root: &Path) -> RepositoryPool {
    DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("test.sqlite")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool")
}

/// Opens a PluginApi bound to `root` so import can exercise `finalize_new_install`.
fn test_plugin_api(root: &Path, pool: &RepositoryPool) -> PluginApi {
    PluginApi::open(
        pool.clone(),
        root.to_path_buf(),
        std::path::PathBuf::from("deno"),
        SystemClock,
        AppEventHub::new().publisher(),
        Arc::new(UserConfigApi::new(pool.clone())),
    )
    .expect("open plugin host")
}

/// Writes a processless Hook `.orax` whose command alias is `rtk` and whose artifact matches
/// `host`.
fn write_hook_orax(path: &Path, identifier: &str, host: &str) {
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"Hook command rewrite\"\n\n[artifact]\ntarget = \"{host}\"\n"
    );
    let config = br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#;
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("orax.toml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.start_file("assets/config.json", options).unwrap();
    writer.write_all(config).unwrap();
    writer.start_file("assets/rtk.exe", options).unwrap();
    writer.write_all(b"MZdummy").unwrap();
    writer.finish().unwrap();
}

/// Marketplace README reads resolve from the source checkout beside the listing's manifest.
#[test]
fn read_plugin_readme_resolves_from_the_marketplace_checkout() {
    with_trace_logging(|| {
        let data_dir = TempDir::new().expect("data dir");
        let pool = test_pool(data_dir.path());
        let api = test_plugin_api(data_dir.path(), &pool);
        let checkout = data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace");
        let digest = "ab".repeat(32);

        let listing_dir = checkout
            .join("registry")
            .join("o")
            .join("ora-space.weather");
        std::fs::create_dir_all(&listing_dir).expect("create listing dir");
        std::fs::write(
            listing_dir.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"ora-space.weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.2.0\"\ndescription = \"Weather plugin\"\nurl = \"https://example.com/weather.orax\"\nsha256 = \"{digest}\"\n"
            ),
        )
        .expect("write listing manifest");
        std::fs::write(
            listing_dir.join("README.md"),
            "# Weather\n\nLive forecasts.",
        )
        .expect("write listing README");

        let response = api
            .read_plugin_readme(ora_contracts::ReadPluginReadmeRequest {
                plugin_id: "official/ora-space.weather".to_string(),
            })
            .expect("read readme");
        assert_eq!(
            response.readme.as_deref(),
            Some("# Weather\n\nLive forecasts.")
        );

        // A listing without a README reports no documentation; an unknown id reports NotFound.
        let silent_dir = checkout.join("registry").join("s").join("ora-space.silent");
        std::fs::create_dir_all(&silent_dir).expect("create silent listing dir");
        std::fs::write(
            silent_dir.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"ora-space.silent\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Silent plugin\"\nurl = \"https://example.com/silent.orax\"\nsha256 = \"{digest}\"\n"
            ),
        )
        .expect("write silent manifest");

        let silent = api
            .read_plugin_readme(ora_contracts::ReadPluginReadmeRequest {
                plugin_id: "official/ora-space.silent".to_string(),
            })
            .expect("read silent readme");
        assert_eq!(silent.readme, None);

        let unknown = api
            .read_plugin_readme(ora_contracts::ReadPluginReadmeRequest {
                plugin_id: "official/absent".to_string(),
            })
            .expect_err("unknown id");
        assert_eq!(
            unknown.to_string(),
            "marketplace plugin was not found in the registry"
        );
    });
}

/// Two Hook packages that share a command alias both stay installed; the second import reports
/// the colliding identity instead of claiming the new package was disabled.
#[test]
fn importing_a_second_hook_with_the_same_command_reports_a_conflict_without_disabling() {
    with_trace_logging(|| {
        let Some(host) = ora_plugin_registry::current_host_target() else {
            eprintln!(
                "skipping Hook command-conflict import: compiled host is not a plugin target"
            );
            return;
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async move {
                let data_dir = TempDir::new().expect("data dir");
                let pool = test_pool(data_dir.path());
                let api = test_plugin_api(data_dir.path(), &pool);
                let first = data_dir.path().join("first.orax");
                let second = data_dir.path().join("second.orax");
                write_hook_orax(&first, "rtk-ai.rtk", host.as_str());
                write_hook_orax(&second, "other.rtk", host.as_str());

                let first_response = api
                    .import(ImportPluginRequest {
                        path: first.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import first Hook");
                assert_eq!(
                    first_response.outcome,
                    InstallOutcome::Installed,
                    "the first Hook must be available without a conflict"
                );

                let second_response = api
                    .import(ImportPluginRequest {
                        path: second.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import second Hook");
                assert_eq!(
                    second_response.outcome,
                    InstallOutcome::InstalledWithCommandConflict {
                        conflict_plugin_id: "local/rtk-ai.rtk".to_string(),
                    }
                );

                let listed = api.list(ListInstalledPluginsRequest {});
                let ids: Vec<&str> = listed
                    .plugins
                    .iter()
                    .map(|plugin| plugin.id.as_str())
                    .collect();
                assert!(
                    ids.contains(&"local/rtk-ai.rtk") && ids.contains(&"local/other.rtk"),
                    "both Hooks must remain installed and available, got {ids:?}"
                );
            });
    });
}
