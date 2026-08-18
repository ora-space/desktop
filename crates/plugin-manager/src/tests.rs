use super::{
    InstalledPluginAgent, MAX_MANIFEST_BYTES, PluginContribution, PluginDiscoveryIssueKind,
    PluginManager, PluginPackageType,
};
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Verifies the complete version-one manifest is retained behind the public interface.
#[test]
fn discovers_complete_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_manifest(
        temp_dir.path(),
        "claude",
        valid_manifest("ora.claude-code", "Claude Code", "0.1.0"),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins().len(), 1);
    let plugin = &manager.installed_plugins()[0];
    assert_eq!(plugin.package_root, package_root);
    assert_eq!(plugin.package_name, "@ora-plugins/claude-code");
    assert_eq!(plugin.version.to_string(), "0.1.0");
    assert_eq!(plugin.package_type, PluginPackageType::Module);
    assert_eq!(plugin.manifest_version, 1);
    assert_eq!(plugin.id, "ora.claude-code");
    assert_eq!(plugin.display_name, "Claude Code");
    assert_eq!(plugin.contributes.kind(), "agent");
    assert_eq!(
        plugin.main,
        PortableRelativePath::parse("dist/index.js").expect("parse expected entrypoint")
    );
    assert_eq!(plugin.engines.ora, ">=0.1.0 <0.2.0");
    assert_eq!(plugin.engines.plugin_api, 1);
    assert_eq!(plugin.engines.bun, ">=1.0.0 <2.0.0");
    assert_eq!(
        plugin.contributes,
        PluginContribution::Agent(InstalledPluginAgent {
            display_name: "Claude Code".to_string(),
            contract_version: 1,
        })
    );
}

/// Verifies a missing plugin root represents an empty installation.
#[test]
fn missing_plugins_root_is_empty() {
    let temp_dir = TempDir::new().unwrap();

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues(), &[]);
}

/// Verifies filesystem enumeration order cannot affect the public snapshot order.
#[test]
fn sorts_plugins_by_identifier_and_accepts_extended_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let mut zeta = valid_manifest("ora.zeta", "工具箱", "1.2.3-alpha.1+build.7");
    zeta["description"] = json!("ignored npm metadata");
    write_manifest(temp_dir.path(), "created-first", zeta);
    write_manifest(
        temp_dir.path(),
        "created-second",
        valid_manifest("ora.alpha", "Alpha", "2.0.0"),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(
        manager
            .installed_plugins()
            .iter()
            .map(|plugin| plugin.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ora.alpha", "ora.zeta"]
    );
    assert_eq!(manager.installed_plugins()[1].display_name, "工具箱");
    assert_eq!(
        manager.installed_plugins()[1].version.to_string(),
        "1.2.3-alpha.1+build.7"
    );
}

/// Verifies malformed packages are isolated while valid siblings remain visible.
#[test]
fn isolates_malformed_and_unsupported_packages() {
    let temp_dir = TempDir::new().unwrap();
    write_manifest(
        temp_dir.path(),
        "valid",
        valid_manifest("ora.valid", "Valid", "1.0.0"),
    );
    write_raw_manifest(temp_dir.path(), "broken", b"{ not-json");
    let mut unsupported = valid_manifest("ora.future", "Future", "1.0.0");
    unsupported["ora"]["manifestVersion"] = json!(2);
    write_manifest(temp_dir.path(), "unsupported", unsupported);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins().len(), 1);
    assert_eq!(manager.installed_plugins()[0].id, "ora.valid");
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![
            PluginDiscoveryIssueKind::InvalidJson,
            PluginDiscoveryIssueKind::InvalidManifest,
        ]
    );
    assert_eq!(
        manager.discovery_issues()[1].field_path(),
        Some("ora.manifestVersion")
    );
}

/// Verifies nested type errors expose the precise Serde field path.
#[test]
fn reports_nested_deserialization_field_path() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = valid_manifest("ora.invalid", "Invalid", "1.0.0");
    manifest["ora"]["engines"]["pluginApi"] = json!("1");
    write_manifest(temp_dir.path(), "invalid", manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues().len(), 1);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidJson
    );
    assert_eq!(
        manager.discovery_issues()[0].field_path(),
        Some("ora.engines.pluginApi")
    );
}

/// Verifies the JSON parser rejects trailing non-whitespace after a valid document.
#[test]
fn rejects_trailing_json_content() {
    let temp_dir = TempDir::new().unwrap();
    let mut bytes = serde_json::to_vec(&valid_manifest("ora.valid", "Valid", "1.0.0")).unwrap();
    bytes.extend_from_slice(b" true");
    write_raw_manifest(temp_dir.path(), "trailing", &bytes);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidJson
    );
}

/// Verifies non-UTF-8 JSON strings fail safely instead of panicking.
#[test]
fn rejects_non_utf8_manifest() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_manifest(temp_dir.path(), "binary", &[0xff, 0xfe, 0xfd]);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidJson
    );
}

/// Verifies bounded reads reject a manifest larger than one MiB.
#[test]
fn rejects_oversized_manifest() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_manifest(
        temp_dir.path(),
        "large",
        &vec![b' '; (MAX_MANIFEST_BYTES + 1) as usize],
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::ManifestTooLarge
    );
}

/// Verifies invalid SemVer forms are delegated to the semver crate and isolated.
#[test]
fn rejects_invalid_package_versions() {
    for version in ["1.0", "1.01.0", "1.0.0-", "18446744073709551616.0.0"] {
        let temp_dir = TempDir::new().unwrap();
        write_manifest(
            temp_dir.path(),
            "invalid-version",
            valid_manifest("ora.invalid", "Invalid", version),
        );

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{version}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some("version"),
            "{version}"
        );
    }
}

/// Verifies all supported constant-valued fields reject incompatible versions and kinds.
#[test]
fn rejects_unsupported_manifest_contract_values() {
    let cases = [
        (vec!["type"], json!("commonjs"), "type"),
        (vec!["ora", "kind"], json!("tool"), "ora.kind"),
        (
            vec!["ora", "engines", "pluginApi"],
            json!(2),
            "ora.engines.pluginApi",
        ),
        (
            vec!["ora", "contributes", "agent", "contractVersion"],
            json!(2),
            "ora.contributes.agent.contractVersion",
        ),
    ];

    for (path, replacement, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_manifest("ora.invalid", "Invalid", "1.0.0");
        replace_path(&mut manifest, &path, replacement);
        write_manifest(temp_dir.path(), "invalid", manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{expected_field}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some(expected_field),
            "{expected_field}"
        );
    }
}

/// Verifies every required string rejects whitespace-only values.
#[test]
fn rejects_empty_required_strings() {
    let cases = [
        (vec!["name"], "name"),
        (vec!["ora", "id"], "ora.id"),
        (vec!["ora", "displayName"], "ora.displayName"),
        (vec!["ora", "main"], "ora.main"),
        (vec!["ora", "engines", "ora"], "ora.engines.ora"),
        (vec!["ora", "engines", "bun"], "ora.engines.bun"),
        (
            vec!["ora", "contributes", "agent", "displayName"],
            "ora.contributes.agent.displayName",
        ),
    ];

    for (path, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_manifest("ora.invalid", "Invalid", "1.0.0");
        replace_path(&mut manifest, &path, json!("   "));
        write_manifest(temp_dir.path(), "invalid", manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{expected_field}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some(expected_field),
            "{expected_field}"
        );
    }
}

/// Verifies entrypoints cannot escape the package directory.
#[test]
fn rejects_unsafe_entrypoints() {
    let mut entrypoints = vec!["../outside.js", "."];
    if cfg!(windows) {
        entrypoints.push("C:\\outside.js");
    } else {
        entrypoints.push("/outside.js");
    }

    for main in entrypoints {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = valid_manifest("ora.invalid", "Invalid", "1.0.0");
        manifest["ora"]["main"] = json!(main);
        write_manifest(temp_dir.path(), "invalid", manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{main}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some("ora.main"),
            "{main}"
        );
    }
}

/// Verifies plugin entrypoints persist the shared portable slash representation.
#[test]
fn normalizes_plugin_entrypoints() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = valid_manifest("ora.normalized", "Normalized", "1.0.0");
    manifest["ora"]["main"] = json!("./dist\\index.js");
    write_manifest(temp_dir.path(), "normalized", manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager.installed_plugins()[0].main,
        PortableRelativePath::parse("dist/index.js").expect("parse expected entrypoint")
    );
}

/// Verifies an entrypoint must already be a regular file when its package is discovered.
#[test]
fn rejects_missing_and_directory_entrypoints() {
    let missing_root = TempDir::new().unwrap();
    let package_root = write_manifest(
        missing_root.path(),
        "missing",
        valid_manifest("ora.missing", "Missing", "1.0.0"),
    );
    fs::remove_file(package_root.join("dist").join("index.js")).unwrap();

    let missing = PluginManager::discover(missing_root.path());

    assert_eq!(missing.installed_plugins(), &[]);
    assert_eq!(missing.discovery_issues()[0].field_path(), Some("ora.main"));

    let directory_root = TempDir::new().unwrap();
    let mut manifest = valid_manifest("ora.directory", "Directory", "1.0.0");
    manifest["ora"]["main"] = json!("dist");
    write_manifest(directory_root.path(), "directory", manifest);

    let directory = PluginManager::discover(directory_root.path());

    assert_eq!(directory.installed_plugins(), &[]);
    assert_eq!(
        directory.discovery_issues()[0].field_path(),
        Some("ora.main")
    );
}

/// Verifies canonical containment rejects an entrypoint symlink that targets outside its package.
#[test]
fn rejects_entrypoint_symlink_escape() {
    let temp_dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let package_root = write_manifest(
        temp_dir.path(),
        "escape",
        valid_manifest("ora.escape", "Escape", "1.0.0"),
    );
    let entrypoint = package_root.join("dist").join("index.js");
    fs::remove_file(&entrypoint).unwrap();
    let outside_entrypoint = outside.path().join("outside.js");
    fs::write(&outside_entrypoint, "export {};\n").unwrap();
    if create_file_symlink(&outside_entrypoint, &entrypoint).is_err() {
        return;
    }

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues()[0].field_path(), Some("ora.main"));
}

/// Verifies an agent package that declares no agent is rejected with a stable field path.
#[test]
fn rejects_agent_package_without_an_agent_contribution() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = valid_manifest("ora.agentless", "Agentless", "1.0.0");
    manifest["ora"]["contributes"] = json!({});
    write_manifest(temp_dir.path(), "agentless", manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].field_path(),
        Some("ora.contributes.agent")
    );
}

/// Verifies duplicate plugin identifiers are diagnosed deterministically.
#[test]
fn rejects_duplicate_plugin_ids() {
    let temp_dir = TempDir::new().unwrap();
    write_manifest(
        temp_dir.path(),
        "01-first-plugin",
        valid_manifest("ora.same", "First", "1.0.0"),
    );
    write_manifest(
        temp_dir.path(),
        "02-second-plugin",
        valid_manifest("ora.same", "Second", "1.0.0"),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins().len(), 1);
    assert_eq!(manager.installed_plugins()[0].display_name, "First");
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![PluginDiscoveryIssueKind::DuplicatePluginId]
    );
}

/// Verifies equal display names remain valid when stable identifiers differ.
#[test]
fn allows_duplicate_display_names() {
    let temp_dir = TempDir::new().unwrap();
    write_manifest(
        temp_dir.path(),
        "one",
        valid_manifest("ora.one", "Same Name", "1.0.0"),
    );
    write_manifest(
        temp_dir.path(),
        "two",
        valid_manifest("ora.two", "Same Name", "1.0.0"),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins().len(), 2);
    assert_eq!(manager.discovery_issues(), &[]);
}

/// Verifies root and manifest filesystem shapes are reported without panics.
#[test]
fn reports_invalid_filesystem_shapes_and_ignores_root_files() {
    let root_file = TempDir::new().unwrap();
    fs::write(root_file.path().join("plugins"), "not a directory").unwrap();
    let root_manager = PluginManager::discover(root_file.path());
    assert_eq!(
        root_manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::RootUnreadable
    );

    let temp_dir = TempDir::new().unwrap();
    fs::create_dir_all(temp_dir.path().join("plugins/missing")).unwrap();
    fs::create_dir_all(temp_dir.path().join("plugins/directory/package.json")).unwrap();
    fs::write(temp_dir.path().join("plugins/ignored.txt"), "ignored").unwrap();
    let manager = PluginManager::discover(temp_dir.path());
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![
            PluginDiscoveryIssueKind::ManifestNotFile,
            PluginDiscoveryIssueKind::MissingManifest,
        ]
    );
}

/// Creates one representative version-one package manifest.
fn valid_manifest(id: &str, display_name: &str, version: &str) -> Value {
    json!({
        "name": "@ora-plugins/claude-code",
        "version": version,
        "type": "module",
        "ora": {
            "manifestVersion": 1,
            "id": id,
            "displayName": display_name,
            "kind": "agent",
            "main": "dist/index.js",
            "engines": {
                "ora": ">=0.1.0 <0.2.0",
                "pluginApi": 1,
                "bun": ">=1.0.0 <2.0.0"
            },
            "contributes": {
                "agent": {
                    "displayName": "Claude Code",
                    "contractVersion": 1
                }
            }
        }
    })
}

/// Writes one JSON manifest below the agreed plugin discovery root.
fn write_manifest(data_dir: &Path, directory: &str, manifest: Value) -> std::path::PathBuf {
    let package_root = data_dir.join("plugins").join(directory);
    fs::create_dir_all(&package_root).unwrap();
    fs::create_dir_all(package_root.join("dist")).unwrap();
    fs::write(package_root.join("dist").join("index.js"), "export {};\n").unwrap();
    fs::write(
        package_root.join("package.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

/// Writes arbitrary bytes as one package manifest.
fn write_raw_manifest(data_dir: &Path, directory: &str, bytes: &[u8]) {
    let package_root = data_dir.join("plugins").join(directory);
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("package.json"), bytes).unwrap();
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

/// Creates a platform-native file symlink when the test environment permits it.
#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Creates a Windows file symlink when Developer Mode or privileges permit it.
#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
