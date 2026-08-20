use super::{
    HostName, InstalledPlugin, InstalledPluginUi, InstalledSurface, InstalledSurfaceSource,
    InstancePolicy, PluginContribution, PluginDiscoveryIssueKind, PluginEngines, PluginManager,
    PluginPackageType, RemoteSiteSource, SurfaceId, WebDataPolicy,
};
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use semver::Version;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use url::Url;

/// Verifies a complete ui manifest is retained as a fully validated, sorted contribution.
#[test]
fn discovers_complete_ui_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = valid_ui_manifest();
    manifest["ora"]["contributes"]["ui"]["surfaces"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "docs",
            "title": "  Docs  ",
            "source": {
                "kind": "remoteSite",
                "entryUrl": "https://developer.huawei.com/consumer",
                "navigation": { "allowHostSuffixes": ["huawei.com"] },
                "webData": "ephemeralIsolated"
            }
        }));
    let package_root = write_manifest(temp_dir.path(), "skillhub", manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager.installed_plugins(),
        &[InstalledPlugin {
            package_root,
            package_name: "@ora-space/skillhub-ui".to_string(),
            version: Version::new(0, 1, 0),
            package_type: PluginPackageType::Module,
            manifest_version: 1,
            id: "ora-space.skillhub".to_string(),
            display_name: "SkillHub".to_string(),
            main: PortableRelativePath::parse("dist/index.js").unwrap(),
            engines: PluginEngines {
                ora: ">= 0.9.0".to_string(),
                plugin_api: 1,
                bun: ">= 1.0.0".to_string(),
            },
            contributes: PluginContribution::Ui(InstalledPluginUi {
                contract_version: 1,
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
        }]
    );
}

/// Verifies every semantic ui rule reports the exact field path from the design.
#[test]
fn rejects_invalid_ui_manifests_with_field_paths() {
    let surface = |id: &str| {
        json!({
            "id": id,
            "title": id,
            "source": {
                "kind": "remoteSite",
                "entryUrl": "https://example.com",
                "navigation": { "allowHosts": ["example.com"] }
            }
        })
    };
    let cases: Vec<(&str, Vec<&str>, Value, &str)> = vec![
        (
            "agent block on ui plugin",
            vec!["ora", "contributes", "agent"],
            json!({ "displayName": "x", "contractVersion": 1 }),
            "ora.contributes.agent",
        ),
        (
            "contract version",
            vec!["ora", "contributes", "ui", "contractVersion"],
            json!(2),
            "ora.contributes.ui.contractVersion",
        ),
        (
            "no surfaces",
            vec!["ora", "contributes", "ui", "surfaces"],
            json!([]),
            "ora.contributes.ui.surfaces",
        ),
        (
            "too many surfaces",
            vec!["ora", "contributes", "ui", "surfaces"],
            Value::Array((0..9).map(|index| surface(&format!("s{index}"))).collect()),
            "ora.contributes.ui.surfaces",
        ),
        (
            "duplicate surface id",
            vec!["ora", "contributes", "ui", "surfaces"],
            json!([surface("same"), surface("same")]),
            "ora.contributes.ui.surfaces[1].id",
        ),
        (
            "surface id not a slug",
            vec!["ora", "contributes", "ui", "surfaces", "0", "id"],
            json!("Market"),
            "ora.contributes.ui.surfaces[0].id",
        ),
        (
            "surface id too long",
            vec!["ora", "contributes", "ui", "surfaces", "0", "id"],
            json!("a".repeat(33)),
            "ora.contributes.ui.surfaces[0].id",
        ),
        (
            "empty title",
            vec!["ora", "contributes", "ui", "surfaces", "0", "title"],
            json!("   "),
            "ora.contributes.ui.surfaces[0].title",
        ),
        (
            "long title",
            vec!["ora", "contributes", "ui", "surfaces", "0", "title"],
            json!("x".repeat(65)),
            "ora.contributes.ui.surfaces[0].title",
        ),
        (
            "control character in title",
            vec!["ora", "contributes", "ui", "surfaces", "0", "title"],
            json!("Skill\u{7}Hub"),
            "ora.contributes.ui.surfaces[0].title",
        ),
        (
            "relative entry url",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "entryUrl",
            ],
            json!("/market"),
            "ora.contributes.ui.surfaces[0].source.entryUrl",
        ),
        (
            "http entry url",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "entryUrl",
            ],
            json!("http://www.skillhub.cn"),
            "ora.contributes.ui.surfaces[0].source.entryUrl",
        ),
        (
            "credentials in entry url",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "entryUrl",
            ],
            json!("https://user:pw@www.skillhub.cn"),
            "ora.contributes.ui.surfaces[0].source.entryUrl",
        ),
        (
            "port in entry url",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "entryUrl",
            ],
            json!("https://www.skillhub.cn:8443"),
            "ora.contributes.ui.surfaces[0].source.entryUrl",
        ),
        (
            "entry host outside allow lists",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "entryUrl",
            ],
            json!("https://evil.example"),
            "ora.contributes.ui.surfaces[0].source.entryUrl",
        ),
        (
            "empty navigation",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "navigation",
            ],
            json!({}),
            "ora.contributes.ui.surfaces[0].source.navigation",
        ),
        (
            "uppercase allow host",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "navigation",
                "allowHosts",
                "1",
            ],
            json!("WWW.skillhub.cn"),
            "ora.contributes.ui.surfaces[0].source.navigation.allowHosts[1]",
        ),
        (
            "scheme in allow host suffix",
            vec![
                "ora",
                "contributes",
                "ui",
                "surfaces",
                "0",
                "source",
                "navigation",
            ],
            json!({ "allowHostSuffixes": ["https://skillhub.cn"] }),
            "ora.contributes.ui.surfaces[0].source.navigation.allowHostSuffixes[0]",
        ),
    ];

    for (name, path, replacement, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_ui_manifest();
        replace_path(&mut manifest, &path, replacement);
        write_manifest(temp_dir.path(), "invalid", manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{name}");
        assert_eq!(
            manager.discovery_issues()[0].kind(),
            PluginDiscoveryIssueKind::InvalidManifest,
            "{name}"
        );
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some(expected_field),
            "{name}"
        );
    }
}

/// Verifies a ui plugin without a ui block, and an agent plugin carrying one, are both rejected.
#[test]
fn rejects_mismatched_kind_and_contribution() {
    let temp_dir = TempDir::new().unwrap();
    let mut missing_ui = valid_ui_manifest();
    missing_ui["ora"]["contributes"] = json!({});
    write_manifest(temp_dir.path(), "a-missing-ui", missing_ui);
    let mut agent_with_ui = valid_ui_manifest();
    agent_with_ui["ora"]["kind"] = json!("agent");
    agent_with_ui["ora"]["contributes"]["agent"] =
        json!({ "displayName": "x", "contractVersion": 1 });
    write_manifest(temp_dir.path(), "b-agent-with-ui", agent_with_ui);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.field_path())
            .collect::<Vec<_>>(),
        vec![Some("ora.contributes.ui"), Some("ora.contributes.ui")]
    );
}

/// Verifies an unsupported surface source kind fails structurally with a precise path.
#[test]
fn rejects_unsupported_surface_source_kind() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = valid_ui_manifest();
    manifest["ora"]["contributes"]["ui"]["surfaces"][0]["source"] =
        json!({ "kind": "panel", "entry": "./panel.js" });
    write_manifest(temp_dir.path(), "panel", manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidJson
    );
    assert_eq!(
        manager.discovery_issues()[0].field_path(),
        Some("ora.contributes.ui.surfaces[0].source.kind")
    );
}

/// Verifies the general plugin id rule shared by every plugin kind.
#[test]
fn rejects_invalid_plugin_ids() {
    let long_segment = "a".repeat(40);
    let cases = [
        "Ora.skillhub",
        "ora.skill_hub",
        "ora.space.skillhub",
        "ora.",
        ".skillhub",
        "ora..skillhub",
        &format!("{long_segment}.{long_segment}"),
    ];
    for id in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_ui_manifest();
        manifest["ora"]["id"] = json!(id);
        write_manifest(temp_dir.path(), "invalid", manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{id}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some("ora.id"),
            "{id}"
        );
    }
}

/// Creates the SkillHub reference manifest from the design document.
fn valid_ui_manifest() -> Value {
    json!({
        "name": "@ora-space/skillhub-ui",
        "version": "0.1.0",
        "type": "module",
        "ora": {
            "manifestVersion": 1,
            "id": "ora-space.skillhub",
            "displayName": "SkillHub",
            "kind": "ui",
            "main": "dist/index.js",
            "engines": { "ora": ">= 0.9.0", "pluginApi": 1, "bun": ">= 1.0.0" },
            "contributes": {
                "ui": {
                    "contractVersion": 1,
                    "surfaces": [{
                        "id": "market",
                        "title": "SkillHub",
                        "instancePolicy": "singleton",
                        "source": {
                            "kind": "remoteSite",
                            "entryUrl": "https://www.skillhub.cn",
                            "navigation": { "allowHosts": ["skillhub.cn", "www.skillhub.cn"] },
                            "webData": "persistentProfile"
                        }
                    }]
                }
            }
        }
    })
}

/// Writes one JSON manifest plus its entrypoint below the plugin discovery root.
fn write_manifest(data_dir: &Path, directory: &str, manifest: Value) -> std::path::PathBuf {
    let package_root = data_dir.join("plugins").join(directory);
    fs::create_dir_all(package_root.join("dist")).unwrap();
    fs::write(package_root.join("dist").join("index.js"), "export {};\n").unwrap();
    fs::write(
        package_root.join("package.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

/// Replaces a nested JSON field, including array indices represented as decimal strings.
fn replace_path(value: &mut Value, path: &[&str], replacement: Value) {
    let mut current = value;
    for segment in &path[..path.len() - 1] {
        current = match segment.parse::<usize>() {
            Ok(index) => &mut current.as_array_mut().unwrap()[index],
            Err(_) => &mut current[*segment],
        };
    }
    let last = path[path.len() - 1];
    match last.parse::<usize>() {
        Ok(index) => current.as_array_mut().unwrap()[index] = replacement,
        Err(_) => current[last] = replacement,
    }
}
