use crate::{Backend, BackendPaths};
use ora_contracts::{
    ImportPluginRequest, InstalledPluginContribution, ListInstalledPluginsRequest,
};
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};

/// Imports locally built Agent archives through the same backend boundary used by Desktop.
///
/// A local `.orax` has no marketplace source, so install assigns the reserved `local` namespace
/// rather than `official`.
#[tokio::test]
async fn locally_built_opencode_and_claude_packages_import_together() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate belongs to the repository");
    let packages = [
        (
            "local/ora-space.opencode",
            "0.6.0",
            repository_root
                .join("opencode-agent")
                .join("dist")
                .join("packages")
                .join("ora-space.opencode-v0.6.0-x86_64-pc-windows-msvc.orax"),
        ),
        (
            "local/ora-space.claude",
            "0.2.0",
            repository_root
                .join("claude-code-agent")
                .join("dist")
                .join("packages")
                .join("ora-space.claude-v0.2.0.orax"),
        ),
    ];
    if packages.iter().any(|(_, _, path)| !path.is_file()) {
        eprintln!("skipping local Agent package import: build artifacts are absent");
        return;
    }
    let temporary = tempfile::tempdir().expect("temporary backend root");
    let backend = Backend::open(BackendPaths {
        app_data_directory: temporary.path().join("data"),
        home_directory: temporary.path().join("home"),
        deno_path: PathBuf::from("deno"),
        relative_path_base: temporary.path().to_path_buf(),
        timezone: "Asia/Shanghai".parse().expect("local timezone"),
    })
    .expect("open backend");
    for (_, _, path) in &packages {
        backend
            .import_plugin(ImportPluginRequest {
                path: path.to_string_lossy().into_owned(),
            })
            .await
            .unwrap_or_else(|error| panic!("import {}: {error:?}", path.display()));
    }
    let installed = backend
        .list_installed_plugins(ListInstalledPluginsRequest {})
        .expect("list imported Agents")
        .plugins;
    let actual = packages
        .iter()
        .map(|(plugin_id, _, _)| {
            let plugin = installed
                .iter()
                .find(|plugin| plugin.id == *plugin_id)
                .unwrap_or_else(|| panic!("imported plugin {plugin_id}"));
            assert!(matches!(
                plugin.contribution,
                InstalledPluginContribution::Agent { .. }
            ));
            (plugin.id.clone(), plugin.version.clone())
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            ("local/ora-space.opencode".to_string(), "0.6.0".to_string()),
            ("local/ora-space.claude".to_string(), "0.2.0".to_string()),
        ]
    );
}
