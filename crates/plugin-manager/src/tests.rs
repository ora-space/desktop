use super::{
    InstalledPluginAgent, MAX_MANIFEST_BYTES, PluginContribution, PluginDiscoveryIssueKind,
    PluginEngines, PluginManager, PluginPackageType,
};
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const VALID_ORAX: &str = "resolver = 1\nname = \"demo\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Demo plugin\"\n";

/// Verifies the complete installed manifest is retained behind the public interface.
#[test]
fn discovers_complete_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_orax_package(
        temp_dir.path(),
        "claude",
        "resolver = 1\nname = \"claude\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"0.1.0\"\ndescription = \"Claude Code\"\nhomepage = \"https://example.com/claude\"\nlicense = \"MIT\"\n",
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins().len(), 1);
    let plugin = &manager.installed_plugins()[0];
    assert_eq!(plugin.package_root, package_root);
    assert_eq!(plugin.package_name, "claude");
    assert_eq!(plugin.version.to_string(), "0.1.0");
    assert_eq!(plugin.package_type, PluginPackageType::Module);
    assert_eq!(plugin.manifest_version, 1);
    assert_eq!(plugin.id, "official/claude");
    assert_eq!(plugin.display_name, "claude");
    assert_eq!(plugin.contributes.kind(), "agent");
    assert_eq!(
        plugin.main,
        PortableRelativePath::parse("main.js").expect("parse expected entrypoint")
    );
    assert_eq!(
        plugin.engines,
        PluginEngines {
            ora: String::new(),
            plugin_api: 1,
            bun: String::new(),
        }
    );
    assert_eq!(
        plugin.contributes,
        PluginContribution::Agent(InstalledPluginAgent {
            display_name: "claude".to_string(),
            contract_version: 1,
        })
    );
}

/// Verifies the installed form tolerates missing `resolver`, `url`, and `sha256`.
#[test]
fn discovers_installed_manifest_without_download_fields() {
    let temp_dir = TempDir::new().unwrap();
    write_orax_package(
        temp_dir.path(),
        "claude",
        "name = \"claude\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"0.1.0\"\ndescription = \"Claude Code\"\n",
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins().len(), 1);
    assert_eq!(manager.installed_plugins()[0].id, "official/claude");
    assert_eq!(manager.installed_plugins()[0].manifest_version, 1);
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
    write_orax_package(
        temp_dir.path(),
        "created-first",
        &manifest_for(
            "zeta",
            "official",
            "agent",
            "1.2.3-alpha.1+build.7",
            "Zeta",
            "https://example.com/zeta",
        ),
    );
    write_orax_package(
        temp_dir.path(),
        "created-second",
        &manifest_for("alpha", "official", "agent", "2.0.0", "Alpha", ""),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(
        manager
            .installed_plugins()
            .iter()
            .map(|plugin| plugin.id.as_str())
            .collect::<Vec<_>>(),
        vec!["official/alpha", "official/zeta"]
    );
    assert_eq!(manager.installed_plugins()[1].display_name, "zeta");
    assert_eq!(
        manager.installed_plugins()[1].version.to_string(),
        "1.2.3-alpha.1+build.7"
    );
}

/// Verifies malformed packages are isolated while valid siblings remain visible.
#[test]
fn isolates_malformed_and_unsupported_packages() {
    let temp_dir = TempDir::new().unwrap();
    write_orax_package(temp_dir.path(), "valid", VALID_ORAX);
    write_raw_orax(temp_dir.path(), "broken", b"{ not-valid-toml");
    write_orax_package(
        temp_dir.path(),
        "unsupported",
        "resolver = 2\nname = \"demo\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Demo\"\n",
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins().len(), 1);
    assert_eq!(manager.installed_plugins()[0].id, "official/demo");
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![
            PluginDiscoveryIssueKind::InvalidToml,
            PluginDiscoveryIssueKind::InvalidManifest,
        ]
    );
    assert_eq!(manager.discovery_issues()[1].field_path(), Some("resolver"));
}

/// Verifies unknown manifest fields are rejected instead of silently accepted.
#[test]
fn rejects_unknown_manifest_fields() {
    let temp_dir = TempDir::new().unwrap();
    let source = format!("{VALID_ORAX}cache = \"/tmp\"\n");
    write_orax_package(temp_dir.path(), "unknown", &source);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidToml
    );
}

/// Verifies non-UTF-8 manifests fail safely instead of panicking.
#[test]
fn rejects_non_utf8_manifest() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_orax(temp_dir.path(), "binary", &[0xff, 0xfe, 0xfd]);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidToml
    );
}

/// Verifies bounded reads reject a manifest larger than one MiB.
#[test]
fn rejects_oversized_manifest() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_orax(
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
        write_orax_package(
            temp_dir.path(),
            "invalid-version",
            &manifest_for("demo", "official", "agent", version, "Demo", ""),
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

/// Verifies one invalid metadata field is reported with its precise manifest path.
#[test]
fn rejects_invalid_manifest_fields() {
    let cases = [
        (
            "bad name!",
            "official",
            "agent",
            "1.0.0",
            "Demo",
            "",
            "name",
        ),
        (
            "demo",
            "community",
            "agent",
            "1.0.0",
            "Demo",
            "",
            "namespace",
        ),
        ("demo", "official", "tool", "1.0.0", "Demo", "", "kind"),
        ("demo", "official", "agent", "1.0", "Demo", "", "version"),
        ("demo", "official", "agent", "1.0.0", "", "", "description"),
        (
            "demo",
            "official",
            "agent",
            "1.0.0",
            "Demo",
            "not a url",
            "homepage",
        ),
    ];

    for (name, namespace, kind, version, description, homepage, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        write_orax_package(
            temp_dir.path(),
            "invalid",
            &manifest_for(name, namespace, kind, version, description, homepage),
        );

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{expected_field}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some(expected_field),
            "{expected_field}"
        );
    }
}

/// Verifies required fields reject whitespace-only values with stable paths.
#[test]
fn rejects_whitespace_only_required_fields() {
    let cases = [
        ("   ", "official", "agent", "1.0.0", "Demo", "name"),
        ("demo", "   ", "agent", "1.0.0", "Demo", "namespace"),
        ("demo", "official", "   ", "1.0.0", "Demo", "kind"),
        ("demo", "official", "agent", "   ", "Demo", "version"),
        ("demo", "official", "agent", "1.0.0", "   ", "description"),
    ];

    for (name, namespace, kind, version, description, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        write_orax_package(
            temp_dir.path(),
            "invalid",
            &manifest_for(name, namespace, kind, version, description, ""),
        );

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{expected_field}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some(expected_field),
            "{expected_field}"
        );
    }
}

/// Verifies a nested field violation exposes the precise dotted manifest path.
#[test]
fn reports_nested_manifest_field_path() {
    let temp_dir = TempDir::new().unwrap();
    write_orax_package(
        temp_dir.path(),
        "invalid",
        "resolver = 1\nname = \"demo\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Demo\"\n[head]\nrepository = \"not a url\"\nbranch = \"main\"\n",
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues().len(), 1);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidManifest
    );
    assert_eq!(
        manager.discovery_issues()[0].field_path(),
        Some("head.repository")
    );
}

/// Verifies an entrypoint must already be a regular file when its package is discovered.
#[test]
fn rejects_missing_and_directory_entrypoints() {
    let missing_root = TempDir::new().unwrap();
    let package_root = write_orax_package(missing_root.path(), "missing", VALID_ORAX);
    fs::remove_file(package_root.join("main.js")).unwrap();

    let missing = PluginManager::discover(missing_root.path());

    assert_eq!(missing.installed_plugins(), &[]);
    assert_eq!(missing.discovery_issues()[0].field_path(), Some("main"));

    let directory_root = TempDir::new().unwrap();
    let package_root = write_orax_package(directory_root.path(), "directory", VALID_ORAX);
    fs::remove_file(package_root.join("main.js")).unwrap();
    fs::create_dir(package_root.join("main.js")).unwrap();

    let directory = PluginManager::discover(directory_root.path());

    assert_eq!(directory.installed_plugins(), &[]);
    assert_eq!(directory.discovery_issues()[0].field_path(), Some("main"));
}

/// Verifies canonical containment rejects an entrypoint symlink that targets outside its package.
#[test]
fn rejects_entrypoint_symlink_escape() {
    let temp_dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let package_root = write_orax_package(temp_dir.path(), "escape", VALID_ORAX);
    let entrypoint = package_root.join("main.js");
    fs::remove_file(&entrypoint).unwrap();
    let outside_entrypoint = outside.path().join("outside.js");
    fs::write(&outside_entrypoint, "export {};\n").unwrap();
    if create_file_symlink(&outside_entrypoint, &entrypoint).is_err() {
        return;
    }

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues()[0].field_path(), Some("main"));
}

/// Verifies the host rejects a kind it cannot run with a stable field path.
#[test]
fn rejects_unsupported_plugin_kind() {
    let temp_dir = TempDir::new().unwrap();
    write_orax_package(
        temp_dir.path(),
        "workbench",
        &manifest_for("demo", "official", "workbench", "1.0.0", "Demo", ""),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::InvalidManifest
    );
    assert_eq!(manager.discovery_issues()[0].field_path(), Some("kind"));
}

/// Verifies duplicate plugin identifiers are diagnosed deterministically.
#[test]
fn rejects_duplicate_plugin_ids() {
    let temp_dir = TempDir::new().unwrap();
    write_orax_package(
        temp_dir.path(),
        "first",
        &manifest_for("demo", "official", "agent", "1.0.0", "First", ""),
    );
    write_orax_package(
        temp_dir.path(),
        "second",
        &manifest_for("demo", "official", "agent", "1.0.0", "Second", ""),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins().len(), 1);
    assert_eq!(manager.installed_plugins()[0].display_name, "demo");
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![PluginDiscoveryIssueKind::DuplicatePluginId]
    );
}

/// Verifies root and manifest filesystem shapes are reported without panics.
#[test]
fn reports_invalid_filesystem_shapes_and_ignores_root_files() {
    let temp_dir = TempDir::new().unwrap();
    fs::create_dir_all(temp_dir.path().join("plugins/installed/missing")).unwrap();
    fs::create_dir_all(
        temp_dir
            .path()
            .join("plugins/installed/directory/orax.toml"),
    )
    .unwrap();
    fs::write(
        temp_dir.path().join("plugins/installed/ignored.txt"),
        "ignored",
    )
    .unwrap();
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

/// Renders one installed `orax.toml` with optional homepage for a single-package fixture.
fn manifest_for(
    name: &str,
    namespace: &str,
    kind: &str,
    version: &str,
    description: &str,
    homepage: &str,
) -> String {
    let homepage = if homepage.is_empty() {
        String::new()
    } else {
        format!("homepage = {homepage:?}\n")
    };
    format!(
        "resolver = 1\nname = {name:?}\nnamespace = {namespace:?}\nkind = {kind:?}\nversion = {version:?}\ndescription = {description:?}\n{homepage}"
    )
}

/// Writes one installed package below `<data>/plugins/installed` with a fixed `main.js` entrypoint.
fn write_orax_package(data_dir: &Path, directory: &str, toml: &str) -> std::path::PathBuf {
    let package_root = data_dir.join("plugins").join("installed").join(directory);
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("main.js"), "export {};\n").unwrap();
    fs::write(package_root.join("orax.toml"), toml).unwrap();
    package_root
}

/// Verifies a package's `logo.svg` is discovered as trusted icon source text.
#[test]
fn discovers_package_logo() {
    let temp_dir = TempDir::new().unwrap();
    let logo = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="8"/></svg>"#;
    let package_root = write_orax_package(temp_dir.path(), "demo", VALID_ORAX);
    fs::write(package_root.join("logo.svg"), logo).unwrap();

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins()[0].logo, Some(logo.to_string()));
}

/// Verifies a package without an icon is discovered cleanly instead of reporting a problem.
#[test]
fn discovers_package_without_a_logo() {
    let temp_dir = TempDir::new().unwrap();
    write_orax_package(temp_dir.path(), "demo", VALID_ORAX);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins()[0].logo, None);
}

/// Verifies an unsafe icon is reported and dropped while the plugin itself stays discovered.
#[test]
fn reports_an_unsafe_logo_without_hiding_the_plugin() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_orax_package(temp_dir.path(), "demo", VALID_ORAX);
    let logo_path = package_root.join("logo.svg");
    fs::write(&logo_path, "<svg><script>evil()</script></svg>").unwrap();

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues().len(), 1);
    let issue = &manager.discovery_issues()[0];
    assert_eq!(issue.path(), logo_path);
    assert_eq!(issue.kind(), PluginDiscoveryIssueKind::UnusableLogo);
    assert_eq!(manager.installed_plugins().len(), 1);
    assert_eq!(manager.installed_plugins()[0].logo, None);
}

/// Writes arbitrary bytes as one installed package manifest.
fn write_raw_orax(data_dir: &Path, directory: &str, bytes: &[u8]) {
    let package_root = data_dir.join("plugins").join("installed").join(directory);
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("orax.toml"), bytes).unwrap();
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
