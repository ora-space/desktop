//! Fixture-backed marketplace/configuration integration tests owned by plugin operations.

use crate::{Backend, test_backend::backend_paths};
use std::fs;
use tempfile::TempDir;

/// Verifies a local Tavily MCP `.orax` import, configuration editor snapshot, and `store.json`
/// persistence for the `apiKey` setting.
#[tokio::test]
async fn tavily_mcp_local_import_and_configuration() {
    use ora_contracts::{
        GetPluginConfigurationRequest, ImportPluginRequest, InstalledPluginContribution,
        ListInstalledPluginsRequest, PluginConfigurationCompleteness, PluginConfigurationSummary,
        PluginSettingValue, SavePluginConfigurationRequest,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    const PLUGIN_ID: &str = "official/ora-space.tavily-search";
    const TEST_API_KEY: &str = "tvly-test-e2e-key";

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
    let orax_archive = workspace_root.join(".tmp/ora-space.tavily-search-v0.1.0.orax");
    if !orax_archive.is_file() {
        eprintln!(
            "skipping Tavily MCP import E2E: missing {}",
            orax_archive.display()
        );
        return;
    }

    let temporary = TempDir::new().expect("create temporary backend directory");
    let data_directory = temporary.path().to_path_buf();
    let backend = Backend::open(backend_paths(&data_directory, &data_directory))
        .expect("open shared backend");

    backend
        .plugins()
        .import(ImportPluginRequest {
            path: orax_archive.to_string_lossy().into_owned(),
        })
        .await
        .expect("import Tavily MCP release");

    let installed = backend
        .plugins()
        .list_installed(ListInstalledPluginsRequest {})
        .expect("list installed plugins")
        .plugins
        .into_iter()
        .find(|plugin| plugin.id == PLUGIN_ID)
        .expect("Tavily MCP plugin is installed");
    assert_eq!(
        installed.contribution,
        InstalledPluginContribution::Mcp,
        "installed plugin contribution"
    );
    assert_eq!(
        installed.configuration,
        PluginConfigurationSummary::Available {
            completeness: PluginConfigurationCompleteness::Incomplete,
        },
        "configuration is incomplete before apiKey is saved"
    );

    let configuration = backend
        .plugins()
        .get_configuration(GetPluginConfigurationRequest {
            plugin_id: PLUGIN_ID.to_string(),
        })
        .expect("load plugin configuration editor")
        .configuration;
    let api_key_setting = configuration
        .settings
        .iter()
        .find(|setting| setting.declaration.id == "apiKey")
        .expect("apiKey setting is declared");
    assert_eq!(api_key_setting.declaration.title, "API key");

    let saved = backend
        .plugins()
        .save_configuration(SavePluginConfigurationRequest {
            preserve_setting_ids: Vec::new(),
            plugin_id: PLUGIN_ID.to_string(),
            expected_revision: configuration.revision,
            declaration_fingerprint: configuration.declaration_fingerprint.clone(),
            values: BTreeMap::from([(
                "apiKey".to_string(),
                PluginSettingValue::String(TEST_API_KEY.to_string()),
            )]),
        })
        .expect("save Tavily apiKey setting")
        .configuration;
    assert_eq!(
        saved.summary,
        PluginConfigurationSummary::Available {
            completeness: PluginConfigurationCompleteness::Complete,
        }
    );

    let store_json = fs::read_to_string(
        data_directory.join("plugins/data/official/ora-space.tavily-search/store.json"),
    )
    .expect("read persisted store.json");
    assert!(
        store_json.contains(TEST_API_KEY),
        "store.json should contain the saved apiKey value"
    );
}

/// Verifies marketplace registry resolution for the Tavily MCP listing without downloading
/// the release archive.
#[test]
fn tavily_mcp_marketplace_manifest_resolves_from_staged_registry() {
    use gitlancer::BranchName;
    use ora_domain::{PluginId, PluginNamespace};
    use ora_plugin_registry::{RegistryIndex, RegistrySource};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
    let marketplace_registry = workspace_root.join(".tmp/marketplace/registry");
    if !marketplace_registry.is_dir() {
        eprintln!(
            "skipping Tavily MCP marketplace manifest E2E: missing {}",
            marketplace_registry.display()
        );
        return;
    }

    let temporary = TempDir::new().expect("create temporary backend directory");
    let marketplace_checkout = temporary
        .path()
        .join("plugins/sources/github.com/ora-space/marketplace");
    fs::create_dir_all(&marketplace_checkout).expect("create marketplace checkout");
    copy_dir_recursive(
        &marketplace_registry,
        &marketplace_checkout.join("registry"),
    )
    .expect("stage marketplace registry");

    let source = RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        PluginNamespace::official(),
        BranchName::new("main"),
        &marketplace_checkout,
    );
    let plugin_id = PluginId::parse("official/ora-space.tavily-search").expect("plugin id");
    let manifest = RegistryIndex::resolve_manifest_all(&[&source], &plugin_id)
        .expect("resolve marketplace manifest")
        .expect("Tavily listing is present in staged registry");
    assert_eq!(
        manifest.url().map(|locator| locator.as_str().to_string()),
        Some(
            "https://github.com/ora-space/tavily-search-mcp/releases/download/v0.1.0/ora-space.tavily-search-v0.1.0.orax"
                .to_string()
        )
    );
    assert_eq!(
        manifest.sha256().map(|digest| digest.to_string()),
        Some("a8b58b0fc0a7c85fe774620682703149b4b6acbaa99303f399309558da282130".to_string())
    );
}

/// Downloads and installs Tavily from a staged marketplace registry. Requires the release URL
/// to be reachable without authentication.
#[tokio::test]
#[ignore = "requires ora-space/tavily-search-mcp release assets to be publicly downloadable"]
async fn tavily_mcp_marketplace_install_and_configuration() {
    use ora_contracts::{
        GetPluginConfigurationRequest, InstallPluginRequest, ListInstalledPluginsRequest,
        PluginConfigurationCompleteness, PluginConfigurationSummary, PluginSettingValue,
        SavePluginConfigurationRequest,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    const PLUGIN_ID: &str = "official/ora-space.tavily-search";
    const TEST_API_KEY: &str = "tvly-test-marketplace-e2e";

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
    let marketplace_registry = workspace_root.join(".tmp/marketplace/registry");
    if !marketplace_registry.is_dir() {
        eprintln!(
            "skipping Tavily MCP marketplace E2E: missing {}",
            marketplace_registry.display()
        );
        return;
    }

    let temporary = TempDir::new().expect("create temporary backend directory");
    let data_directory = temporary.path().to_path_buf();
    let marketplace_checkout =
        data_directory.join("plugins/sources/github.com/ora-space/marketplace");
    fs::create_dir_all(&marketplace_checkout).expect("create marketplace checkout");
    copy_dir_recursive(
        &marketplace_registry,
        &marketplace_checkout.join("registry"),
    )
    .expect("stage marketplace registry");

    let backend = Backend::open(backend_paths(&data_directory, &data_directory))
        .expect("open shared backend");

    backend
        .plugins()
        .install(InstallPluginRequest {
            plugin_id: PLUGIN_ID.to_string(),
        })
        .await
        .unwrap_or_else(|error| {
            panic!(
                "install Tavily MCP from staged marketplace registry: {error:?}. \
                 If the release URL returns 404, ensure ora-space/tavily-search-mcp is public."
            );
        });

    let installed = backend
        .plugins()
        .list_installed(ListInstalledPluginsRequest {})
        .expect("list installed plugins")
        .plugins
        .into_iter()
        .find(|plugin| plugin.id == PLUGIN_ID)
        .expect("Tavily MCP plugin is installed after marketplace install");
    assert_eq!(
        installed.configuration,
        PluginConfigurationSummary::Available {
            completeness: PluginConfigurationCompleteness::Incomplete,
        }
    );

    let configuration = backend
        .plugins()
        .get_configuration(GetPluginConfigurationRequest {
            plugin_id: PLUGIN_ID.to_string(),
        })
        .expect("load plugin configuration editor")
        .configuration;
    backend
        .plugins()
        .save_configuration(SavePluginConfigurationRequest {
            preserve_setting_ids: Vec::new(),
            plugin_id: PLUGIN_ID.to_string(),
            expected_revision: configuration.revision,
            declaration_fingerprint: configuration.declaration_fingerprint.clone(),
            values: BTreeMap::from([(
                "apiKey".to_string(),
                PluginSettingValue::String(TEST_API_KEY.to_string()),
            )]),
        })
        .expect("save Tavily apiKey after marketplace install");

    let store_json = fs::read_to_string(
        data_directory.join("plugins/data/official/ora-space.tavily-search/store.json"),
    )
    .expect("read persisted store.json");
    assert!(
        store_json.contains(TEST_API_KEY),
        "store.json should contain the saved apiKey value"
    );
}

/// When `ORA_E2E_PLUGIN_DATA` points at a live Desktop plugin home, verifies Tavily settings
/// persistence against the real on-disk layout.
#[tokio::test]
async fn tavily_mcp_save_configuration_in_desktop_plugin_home() {
    use ora_contracts::{
        GetPluginConfigurationRequest, ListInstalledPluginsRequest,
        PluginConfigurationCompleteness, PluginConfigurationSummary, PluginSettingValue,
        SavePluginConfigurationRequest,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    const PLUGIN_ID: &str = "official/ora-space.tavily-search";
    const TEST_API_KEY: &str = "tvly-desktop-e2e-key";

    let Ok(data_directory) = std::env::var("ORA_E2E_PLUGIN_DATA") else {
        return;
    };
    let data_directory = PathBuf::from(data_directory);
    let plugin_data_directory = data_directory.clone();
    if !data_directory.is_dir() {
        eprintln!(
            "skipping desktop-home Tavily E2E: {} is not a directory",
            data_directory.display()
        );
        return;
    }

    let temporary = TempDir::new().expect("create temporary backend directory");
    let backend = Backend::open(backend_paths(temporary.path(), &data_directory))
        .expect("open shared backend");

    let installed = backend
        .plugins()
        .list_installed(ListInstalledPluginsRequest {})
        .expect("list installed plugins")
        .plugins
        .into_iter()
        .find(|plugin| plugin.id == PLUGIN_ID)
        .expect("Tavily MCP plugin is installed in desktop plugin home");
    assert!(
        matches!(
            installed.configuration,
            PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Incomplete,
            } | PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Complete,
            }
        ),
        "configuration should be available after the dotted-name store-path fix"
    );

    let configuration = backend
        .plugins()
        .get_configuration(GetPluginConfigurationRequest {
            plugin_id: PLUGIN_ID.to_string(),
        })
        .expect("load plugin configuration editor")
        .configuration;
    if matches!(
        configuration.summary,
        PluginConfigurationSummary::Available {
            completeness: PluginConfigurationCompleteness::Complete,
        }
    ) {
        return;
    }
    backend
        .plugins()
        .save_configuration(SavePluginConfigurationRequest {
            preserve_setting_ids: Vec::new(),
            plugin_id: PLUGIN_ID.to_string(),
            expected_revision: configuration.revision,
            declaration_fingerprint: configuration.declaration_fingerprint.clone(),
            values: BTreeMap::from([(
                "apiKey".to_string(),
                PluginSettingValue::String(TEST_API_KEY.to_string()),
            )]),
        })
        .expect("save Tavily apiKey in desktop plugin home");

    let store_json = fs::read_to_string(
        plugin_data_directory.join("plugins/data/official/ora-space.tavily-search/store.json"),
    )
    .expect("read persisted store.json");
    assert!(
        store_json.contains(TEST_API_KEY),
        "store.json should contain the saved apiKey value"
    );
}

/// Recursively copies one directory tree for marketplace registry staging in tests.
fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &destination)?;
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}
