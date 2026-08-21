use super::tests::{replace_path, write_manifest};
use super::{
    HostName, InstalledPlugin, InstalledPluginUi, InstalledSurface, InstalledSurfaceSource,
    InstancePolicy, PanelSource, PluginContribution, PluginDiscoveryIssueKind, PluginManager,
    RemoteSiteSource, SurfaceId, WebDataPolicy,
};
use ora_domain::PluginId;
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use semver::Version;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use toml::Value;
use url::Url;

const NAMESPACE: &str = "official";
const SKILLHUB: &str = "ora-space.skillhub";
const HELLO_PANEL: &str = "ora-space.hello-panel";

/// Verifies a complete ui manifest is retained as a fully validated, sorted contribution and
/// that a ui plugin's display name is its plugin name.
#[test]
fn discovers_complete_ui_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = valid_ui_manifest();
    manifest["ui"]["surfaces"].as_array_mut().unwrap().push(
        toml::from_str(
            r#"
id = "docs"
title = "  Docs  "
[source]
kind = "remote_site"
entry = "https://developer.huawei.com/consumer"
host_suffixes = ["huawei.com"]
[web_data]
mode = "ephemeral"
"#,
        )
        .unwrap(),
    );
    let package_root = write_manifest(temp_dir.path(), SKILLHUB, manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager.installed_plugins(),
        &[InstalledPlugin {
            package_root,
            id: PluginId::new(NAMESPACE, SKILLHUB).unwrap(),
            version: Version::new(0, 1, 0),
            display_name: SKILLHUB.to_string(),
            description: "Ora Space SkillHub surface".to_string(),
            homepage: Some("https://github.com/ora-space/ui-plugins".to_string()),
            license: Some("Apache-2.0".to_string()),
            main: PortableRelativePath::parse("main.js").unwrap(),
            contributes: PluginContribution::Ui(InstalledPluginUi {
                surfaces: vec![
                    InstalledSurface {
                        id: SurfaceId::parse("docs").unwrap(),
                        title: "Docs".to_string(),
                        instance_policy: InstancePolicy::Singleton,
                        source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                            entry_url: Url::parse("https://developer.huawei.com/consumer").unwrap(),
                            allow_hosts: vec![],
                            allow_host_suffixes: vec![HostName::parse("huawei.com").unwrap()],
                            web_data: WebDataPolicy::EphemeralIsolated,
                        }),
                    },
                    InstalledSurface {
                        id: SurfaceId::parse("market").unwrap(),
                        title: "SkillHub".to_string(),
                        instance_policy: InstancePolicy::Singleton,
                        source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                            entry_url: Url::parse("https://www.skillhub.cn").unwrap(),
                            allow_hosts: vec![
                                HostName::parse("skillhub.cn").unwrap(),
                                HostName::parse("www.skillhub.cn").unwrap(),
                            ],
                            allow_host_suffixes: vec![],
                            web_data: WebDataPolicy::PersistentProfile,
                        }),
                    },
                ],
            }),
            logo: None,
        }]
    );
}

/// Verifies every semantic ui rule reports the exact field path from the design.
#[test]
fn rejects_invalid_ui_manifests_with_field_paths() {
    let surface = |id: &str| -> Value {
        toml::from_str(&format!(
            r#"
id = "{id}"
title = "{id}"
[source]
kind = "remote_site"
entry = "https://example.com"
hosts = ["example.com"]
"#
        ))
        .unwrap()
    };
    let cases: Vec<(&str, Vec<&str>, Value, &str)> = vec![
        (
            "no surfaces",
            vec!["ui", "surfaces"],
            Value::Array(vec![]),
            "ui.surfaces",
        ),
        (
            "too many surfaces",
            vec!["ui", "surfaces"],
            Value::Array((0..9).map(|index| surface(&format!("s{index}"))).collect()),
            "ui.surfaces",
        ),
        (
            "duplicate surface id",
            vec!["ui", "surfaces"],
            Value::Array(vec![surface("same"), surface("same")]),
            "ui.surfaces[1].id",
        ),
        (
            "surface id not a slug",
            vec!["ui", "surfaces", "0", "id"],
            Value::from("Market"),
            "ui.surfaces[0].id",
        ),
        (
            "surface id too long",
            vec!["ui", "surfaces", "0", "id"],
            Value::from("a".repeat(33)),
            "ui.surfaces[0].id",
        ),
        (
            "empty title",
            vec!["ui", "surfaces", "0", "title"],
            Value::from("   "),
            "ui.surfaces[0].title",
        ),
        (
            "long title",
            vec!["ui", "surfaces", "0", "title"],
            Value::from("x".repeat(65)),
            "ui.surfaces[0].title",
        ),
        (
            "control character in title",
            vec!["ui", "surfaces", "0", "title"],
            Value::from("Skill\u{7}Hub"),
            "ui.surfaces[0].title",
        ),
        (
            "multiple instances unsupported",
            vec!["ui", "surfaces", "0", "instances"],
            Value::from("multiple"),
            "ui.surfaces[0].instances",
        ),
        (
            "unknown instances policy",
            vec!["ui", "surfaces", "0", "instances"],
            Value::from("many"),
            "ui.surfaces[0].instances",
        ),
        (
            "unknown web data mode",
            vec!["ui", "surfaces", "0", "web_data", "mode"],
            Value::from("persistent_profile"),
            "ui.surfaces[0].web_data.mode",
        ),
        (
            "relative entry url",
            vec!["ui", "surfaces", "0", "source", "entry"],
            Value::from("/market"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "http entry url",
            vec!["ui", "surfaces", "0", "source", "entry"],
            Value::from("http://www.skillhub.cn"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "credentials in entry url",
            vec!["ui", "surfaces", "0", "source", "entry"],
            Value::from("https://user:pw@www.skillhub.cn"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "port in entry url",
            vec!["ui", "surfaces", "0", "source", "entry"],
            Value::from("https://www.skillhub.cn:8443"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "entry host outside allow lists",
            vec!["ui", "surfaces", "0", "source", "entry"],
            Value::from("https://evil.example"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "empty allow lists",
            vec!["ui", "surfaces", "0", "source", "hosts"],
            Value::Array(vec![]),
            "ui.surfaces[0].source",
        ),
        (
            "uppercase allow host",
            vec!["ui", "surfaces", "0", "source", "hosts", "1"],
            Value::from("WWW.skillhub.cn"),
            "ui.surfaces[0].source.hosts[1]",
        ),
        (
            "scheme in allow host suffix",
            vec!["ui", "surfaces", "0", "source", "host_suffixes"],
            Value::Array(vec![Value::from("https://skillhub.cn")]),
            "ui.surfaces[0].source.host_suffixes[0]",
        ),
    ];

    for (name, path, replacement, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_ui_manifest();
        replace_path(&mut manifest, &path, replacement);
        write_manifest(temp_dir.path(), SKILLHUB, manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(
            (
                manager.installed_plugins().len(),
                manager.discovery_issues()[0].kind(),
                manager.discovery_issues()[0].field_path(),
            ),
            (
                0,
                PluginDiscoveryIssueKind::InvalidManifest,
                Some(expected_field)
            ),
            "{name}"
        );
    }
}

/// Verifies a ui plugin without a ui section, and an agent plugin carrying one, are both rejected.
#[test]
fn rejects_mismatched_kind_and_section() {
    let temp_dir = TempDir::new().unwrap();
    let mut missing_ui = valid_ui_manifest();
    missing_ui.as_table_mut().unwrap().remove("ui");
    missing_ui["name"] = Value::from("a-missing-ui");
    write_manifest(temp_dir.path(), "a-missing-ui", missing_ui);
    let mut agent_with_ui = valid_ui_manifest();
    agent_with_ui["name"] = Value::from("b-agent-with-ui");
    agent_with_ui["kind"] = Value::from("agent");
    write_manifest(temp_dir.path(), "b-agent-with-ui", agent_with_ui);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.field_path())
            .collect::<Vec<_>>(),
        vec![Some("ui"), Some("ui")]
    );
}

/// Verifies structural source errors fail as invalid TOML with the precise TOML path: an
/// unsupported source kind and a field of the other source form.
#[test]
fn rejects_structurally_invalid_surface_sources() {
    let cases: Vec<(&str, Vec<&str>, Value, &str)> = vec![
        (
            "unsupported source kind",
            vec!["ui", "surfaces", "0", "source"],
            toml::from_str("kind = \"native_view\"\nentry = \"./panel.js\"").unwrap(),
            "ui.surfaces[0].source.kind",
        ),
        (
            "panel field on remote site",
            vec!["ui", "surfaces", "0", "source", "root"],
            Value::from("ui"),
            "ui.surfaces[0].source",
        ),
    ];
    for (label, path, replacement, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_ui_manifest();
        replace_path(&mut manifest, &path, replacement);
        write_manifest(temp_dir.path(), SKILLHUB, manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(
            (
                manager.installed_plugins().len(),
                manager.discovery_issues()[0].kind(),
                manager.discovery_issues()[0].field_path(),
            ),
            (
                0,
                PluginDiscoveryIssueKind::InvalidToml,
                Some(expected_field)
            ),
            "{label}: {}",
            manager.discovery_issues()[0]
        );
    }
}

/// Verifies a panel surface resolves its asset directory canonically and keeps the entry
/// relative to it, so the asset handler can use the directory as a containment root.
#[test]
fn discovers_panel_surface() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_manifest(temp_dir.path(), HELLO_PANEL, valid_panel_manifest());
    write_panel_assets(&package_root);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager.installed_plugins()[0].contributes,
        PluginContribution::Ui(InstalledPluginUi {
            surfaces: vec![InstalledSurface {
                id: SurfaceId::parse("counter").unwrap(),
                title: "Hello Panel".to_string(),
                instance_policy: InstancePolicy::Singleton,
                source: InstalledSurfaceSource::Panel(PanelSource {
                    asset_root: package_root.join("ui").canonicalize().unwrap(),
                    entry: PortableRelativePath::parse("index.html").unwrap(),
                }),
            }],
        })
    );
}

/// Verifies every panel rule reports the field that violated it.
#[test]
fn rejects_invalid_panel_manifests_with_field_paths() {
    let cases: Vec<(&str, Vec<&str>, Value, &str)> = vec![
        (
            "root escapes the package",
            vec!["source", "root"],
            Value::from("../ui"),
            "ui.surfaces[0].source.root",
        ),
        (
            "root is the package itself",
            vec!["source", "root"],
            Value::from("."),
            "ui.surfaces[0].source.root",
        ),
        (
            "root does not exist",
            vec!["source", "root"],
            Value::from("missing"),
            "ui.surfaces[0].source.root",
        ),
        (
            "root is a file",
            vec!["source", "root"],
            Value::from("ui/index.html"),
            "ui.surfaces[0].source.root",
        ),
        (
            "entry is not html",
            vec!["source", "entry"],
            Value::from("app.js"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "entry does not exist",
            vec!["source", "entry"],
            Value::from("other.html"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "entry escapes the root",
            vec!["source", "entry"],
            Value::from("../package.html"),
            "ui.surfaces[0].source.entry",
        ),
        (
            "web data declared on a panel",
            vec!["web_data"],
            toml::from_str("mode = \"persistent\"").unwrap(),
            "ui.surfaces[0].web_data",
        ),
    ];
    for (label, path, replacement, expected_path) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_panel_manifest();
        replace_path(&mut manifest["ui"]["surfaces"][0], &path, replacement);
        let package_root = write_manifest(temp_dir.path(), HELLO_PANEL, manifest);
        write_panel_assets(&package_root);
        fs::write(package_root.join("package.html"), "<html></html>\n").unwrap();

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(
            (
                manager.installed_plugins().len(),
                manager.discovery_issues()[0].kind(),
                manager.discovery_issues()[0].field_path(),
            ),
            (
                0,
                PluginDiscoveryIssueKind::InvalidManifest,
                Some(expected_path)
            ),
            "{label}"
        );
    }
}

/// Creates the SkillHub reference manifest from the design document.
fn valid_ui_manifest() -> Value {
    toml::from_str(
        r#"
resolver = 1
name = "ora-space.skillhub"
namespace = "official"
kind = "ui"
version = "0.1.0"
description = "Ora Space SkillHub surface"
homepage = "https://github.com/ora-space/ui-plugins"
license = "Apache-2.0"
[dependencies]
ora = ">= 0.9.0"

[[ui.surfaces]]
id = "market"
title = "SkillHub"
instances = "singleton"

[ui.surfaces.source]
kind = "remote_site"
entry = "https://www.skillhub.cn"
hosts = ["skillhub.cn", "www.skillhub.cn"]

[ui.surfaces.web_data]
mode = "persistent"
"#,
    )
    .unwrap()
}

/// Builds the manifest of a ui plugin whose only surface is a package-shipped panel.
fn valid_panel_manifest() -> Value {
    let mut manifest = valid_ui_manifest();
    manifest["name"] = Value::from(HELLO_PANEL);
    manifest["ui"]["surfaces"] = Value::Array(vec![
        toml::from_str(
            r#"
id = "counter"
title = "Hello Panel"
[source]
kind = "panel"
root = "ui"
entry = "index.html"
"#,
        )
        .unwrap(),
    ]);
    manifest
}

/// Writes the panel page and script a valid panel manifest points at.
fn write_panel_assets(package_root: &Path) {
    fs::create_dir_all(package_root.join("ui")).unwrap();
    fs::write(
        package_root.join("ui").join("index.html"),
        "<html></html>\n",
    )
    .unwrap();
    fs::write(package_root.join("ui").join("app.js"), "export {};\n").unwrap();
}
