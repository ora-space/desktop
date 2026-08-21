//! Parses the manifests shipped by the sample ui plugins (`ora-space/ui-plugins`) through the
//! real discovery path, so a change to the manifest grammar that would break the published
//! samples is caught here instead of at the user's first install.
//!
//! The fixtures under `fixtures/ui-plugins/` are verbatim copies of each sample's `orax.toml`;
//! the test writes them into the installed layout next to the files they reference.

use super::{
    HostName, InstalledPlugin, InstalledPluginUi, InstalledSurface, InstalledSurfaceSource,
    InstancePolicy, PanelSource, PluginContribution, PluginManager, RemoteSiteSource, SurfaceId,
    WebDataPolicy,
};
use ora_domain::PluginId;
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use semver::Version;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const NAMESPACE: &str = "official";

/// One sample plugin: its fixture directory and the identity its manifest must carry.
struct Sample {
    fixture: &'static str,
    name: &'static str,
    ships_panel: bool,
}

const SAMPLES: [Sample; 3] = [
    Sample {
        fixture: "skillhub",
        name: "ora-space.skillhub",
        ships_panel: false,
    },
    Sample {
        fixture: "huawei-agent-center",
        name: "ora-space.huawei-agent-center",
        ships_panel: false,
    },
    Sample {
        fixture: "hello-panel",
        name: "ora-space.hello-panel",
        ships_panel: true,
    },
];

/// Installs one sample the way `deno task install` does: the manifest, the bundled entrypoint,
/// and the panel page when the sample ships one. Returns the package directory.
fn install_sample(data_dir: &Path, sample: &Sample) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("ui-plugins")
        .join(sample.fixture)
        .join("orax.toml");
    let package_root = super::installed_root(data_dir).join(sample.name);
    fs::create_dir_all(&package_root).unwrap();
    fs::copy(fixture, package_root.join("orax.toml")).unwrap();
    fs::write(package_root.join("main.js"), "export {};\n").unwrap();
    if sample.ships_panel {
        fs::create_dir_all(package_root.join("ui")).unwrap();
        fs::write(
            package_root.join("ui").join("index.html"),
            "<html></html>\n",
        )
        .unwrap();
    }
    package_root
}

/// Builds the expected remote-site surface shared by the two marketplace samples.
fn remote_site(
    title: &str,
    entry: &str,
    hosts: &[&str],
    host_suffixes: &[&str],
) -> InstalledSurface {
    InstalledSurface {
        id: SurfaceId::parse("market").unwrap(),
        title: title.to_string(),
        instance_policy: InstancePolicy::Singleton,
        source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
            entry_url: entry.parse().unwrap(),
            allow_hosts: hosts.iter().map(|h| HostName::parse(h).unwrap()).collect(),
            allow_host_suffixes: host_suffixes
                .iter()
                .map(|h| HostName::parse(h).unwrap())
                .collect(),
            web_data: WebDataPolicy::PersistentProfile,
        }),
    }
}

/// Every sample manifest discovers without issues and yields exactly the contribution its
/// README documents.
#[test]
fn sample_ui_plugin_manifests_discover_cleanly() {
    let temp_dir = TempDir::new().unwrap();
    let roots: Vec<PathBuf> = SAMPLES
        .iter()
        .map(|sample| install_sample(temp_dir.path(), sample))
        .collect();

    let manager = PluginManager::discover(temp_dir.path());

    let expected = |index: usize, description: &str, surface: InstalledSurface| InstalledPlugin {
        package_root: roots[index].clone(),
        id: PluginId::new(NAMESPACE, SAMPLES[index].name).unwrap(),
        version: Version::new(0, 1, 0),
        display_name: SAMPLES[index].name.to_string(),
        description: description.to_string(),
        homepage: Some("https://github.com/ora-space/ui-plugins".to_string()),
        license: Some("Apache-2.0".to_string()),
        main: PortableRelativePath::parse("main.js").unwrap(),
        contributes: PluginContribution::Ui(InstalledPluginUi {
            surfaces: vec![surface],
        }),
        logo: None,
    };
    assert_eq!(manager.discovery_issues(), &[]);
    // Discovery sorts by plugin id, which is alphabetical here.
    assert_eq!(
        manager.installed_plugins(),
        &[
            expected(
                2,
                "Ora Space Hello Panel sample surface",
                InstalledSurface {
                    id: SurfaceId::parse("counter").unwrap(),
                    title: "Hello Panel".to_string(),
                    instance_policy: InstancePolicy::Singleton,
                    source: InstalledSurfaceSource::Panel(PanelSource {
                        asset_root: roots[2].canonicalize().unwrap().join("ui"),
                        entry: PortableRelativePath::parse("index.html").unwrap(),
                    }),
                },
            ),
            expected(
                1,
                "Ora Space Huawei Agent Center surface",
                remote_site(
                    "Huawei Agent Center",
                    "https://ai.edevops.huawei.com/mcp/projects",
                    &[],
                    &["huawei.com"],
                ),
            ),
            expected(
                0,
                "Ora Space SkillHub surface",
                remote_site(
                    "SkillHub",
                    "https://www.skillhub.cn",
                    &["skillhub.cn", "www.skillhub.cn"],
                    &[],
                ),
            ),
        ]
    );
}
