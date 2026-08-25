use super::{
    InstalledPlugin, InstalledPluginAgent, MAX_MANIFEST_BYTES, PluginContribution,
    PluginDiscoveryIssueKind, PluginManager,
};
use ora_domain::PluginId;
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use semver::Version;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use toml::Value;

/// Identity of the agent fixture every test starts from.
const NAMESPACE: &str = "official";
const NAME: &str = "ora.claude-code";

/// Verifies the complete manifest is retained behind the public interface.
#[test]
fn discovers_complete_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_manifest(temp_dir.path(), NAME, agent_manifest());

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager.installed_plugins(),
        &[InstalledPlugin {
            package_root,
            id: PluginId::new(NAMESPACE, NAME).unwrap(),
            version: Version::new(0, 1, 0),
            display_name: NAME.to_string(),
            description: "Claude Code agent".to_string(),
            homepage: Some("https://example.com/claude-code".to_string()),
            license: Some("Apache-2.0".to_string()),
            contributes: PluginContribution::Agent(InstalledPluginAgent {
                display_name: NAME.to_string(),
                entrypoint: PortableRelativePath::parse("main.js").unwrap(),
            }),
            logo: None,
        }]
    );
}

/// Verifies a Skill plugin is static but must contain at least one complete Skill package.
#[test]
fn discovers_skill_plugin_with_required_skill_assets() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = agent_manifest();
    manifest["identifier"] = Value::from("ora.skill-pack");
    manifest["kind"] = Value::from("skill");
    let package_root = write_manifest(temp_dir.path(), "ora.skill-pack", manifest);
    fs::remove_file(package_root.join("main.js")).unwrap();
    for name in ["review", "testing"] {
        let skill_root = package_root.join("assets/skills").join(name);
        fs::create_dir_all(&skill_root).unwrap();
        fs::write(
            skill_root.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Example\n---\n"),
        )
        .unwrap();
    }

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins().len(), 1);
    let PluginContribution::Skill(descriptor) = &manager.installed_plugins()[0].contributes else {
        panic!("expected Skill contribution");
    };
    assert_eq!(
        descriptor
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>(),
        vec!["review", "testing"]
    );
}

/// Verifies missing, empty, or incomplete `assets/skills` trees are not discovered.
#[test]
fn rejects_skill_plugins_without_complete_skill_assets() {
    let mut managers = Vec::new();
    for case in ["missing", "empty", "incomplete"] {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = agent_manifest();
        manifest["identifier"] = Value::from(format!("ora.skill-{case}"));
        manifest["kind"] = Value::from("skill");
        let package_root = write_manifest(temp_dir.path(), case, manifest);
        fs::remove_file(package_root.join("main.js")).unwrap();
        if case != "missing" {
            fs::create_dir_all(package_root.join("assets/skills")).unwrap();
        }
        if case == "incomplete" {
            fs::create_dir_all(package_root.join("assets/skills/review")).unwrap();
        }

        let manager = PluginManager::discover(temp_dir.path());
        managers.push((temp_dir, manager));
    }

    for (_, manager) in managers {
        assert_eq!(manager.installed_plugins(), &[]);
        assert_eq!(manager.discovery_issues()[0].field_path(), Some("skill"));
    }
}
/// Verifies a missing installed root represents an empty installation.
#[test]
fn missing_installed_root_is_empty() {
    let temp_dir = TempDir::new().unwrap();

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues(), &[]);
}

/// Verifies filesystem enumeration order and directory names cannot affect the public snapshot
/// order, and that optional metadata may be omitted.
#[test]
fn sorts_plugins_by_identifier_and_accepts_minimal_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let mut zeta = agent_manifest();
    zeta["identifier"] = Value::from("ora.zeta");
    zeta["version"] = Value::from("1.2.3-alpha.1+build.7");
    zeta.as_table_mut().unwrap().remove("homepage");
    zeta.as_table_mut().unwrap().remove("license");
    zeta.as_table_mut().unwrap().remove("dependencies");
    // The directory is deliberately named against sort order: identity comes from the manifest.
    write_manifest(temp_dir.path(), "a-directory", zeta);
    let mut alpha = agent_manifest();
    alpha["identifier"] = Value::from("ora.alpha");
    alpha["version"] = Value::from("2.0.0");
    write_manifest(temp_dir.path(), "z-directory", alpha);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager
            .installed_plugins()
            .iter()
            .map(|plugin| {
                (
                    plugin.id.canonical(),
                    plugin.version.to_string(),
                    plugin.homepage.clone(),
                    plugin.license.clone(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                "official/ora.alpha".to_string(),
                "2.0.0".to_string(),
                Some("https://example.com/claude-code".to_string()),
                Some("Apache-2.0".to_string()),
            ),
            (
                "official/ora.zeta".to_string(),
                "1.2.3-alpha.1+build.7".to_string(),
                None,
                None,
            ),
        ]
    );
}

/// Verifies two directories claiming one plugin id keep the first in path order and report the
/// second, so a stray copy cannot shadow the installed package.
#[test]
fn reports_duplicate_plugin_ids() {
    let temp_dir = TempDir::new().unwrap();
    let first = write_manifest(temp_dir.path(), "a-copy", agent_manifest());
    write_manifest(temp_dir.path(), "b-copy", agent_manifest());

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(
        manager
            .installed_plugins()
            .iter()
            .map(|plugin| plugin.package_root.clone())
            .collect::<Vec<_>>(),
        vec![first]
    );
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| (issue.kind(), issue.field_path()))
            .collect::<Vec<_>>(),
        vec![(
            PluginDiscoveryIssueKind::DuplicatePluginId,
            Some("identifier")
        )]
    );
}

/// Verifies malformed packages are isolated while valid siblings remain visible.
#[test]
fn isolates_malformed_and_unsupported_packages() {
    let temp_dir = TempDir::new().unwrap();
    write_manifest(temp_dir.path(), "ora.valid", named("ora.valid"));
    write_raw_manifest(temp_dir.path(), "ora.broken", b"identifier = [");
    let mut unsupported = named("ora.future");
    unsupported["resolver"] = Value::from(2);
    write_manifest(temp_dir.path(), "ora.future", unsupported);
    // A workbench package without its page is a host-side (not schema) rejection.
    let mut workbench = named("ora.workbench");
    workbench["kind"] = Value::from("workbench");
    write_manifest(temp_dir.path(), "ora.workbench", workbench);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(
        manager
            .installed_plugins()
            .iter()
            .map(|plugin| plugin.id.canonical())
            .collect::<Vec<_>>(),
        vec!["official/ora.valid".to_string()]
    );
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| (issue.kind(), issue.field_path()))
            .collect::<Vec<_>>(),
        vec![
            (PluginDiscoveryIssueKind::InvalidToml, None),
            (PluginDiscoveryIssueKind::InvalidManifest, Some("resolver")),
            (PluginDiscoveryIssueKind::InvalidManifest, Some("workbench")),
        ]
    );
}

/// Verifies nested type errors and unknown fields expose the precise TOML path as structural
/// (`invalid_toml`) issues, while unknown enum spellings are semantic field errors.
#[test]
fn reports_structural_errors_with_field_paths() {
    let cases: Vec<(&str, Vec<&str>, Value, PluginDiscoveryIssueKind, &str)> = vec![
        (
            "nested type error",
            vec!["dependencies", "ora"],
            Value::from(1),
            PluginDiscoveryIssueKind::InvalidToml,
            "dependencies.ora",
        ),
        (
            "unknown top-level field",
            vec!["engines"],
            Value::from("bun"),
            PluginDiscoveryIssueKind::InvalidToml,
            "engines",
        ),
        (
            "unknown nested field",
            vec!["dependencies", "bun"],
            Value::from("1"),
            PluginDiscoveryIssueKind::InvalidToml,
            "dependencies.bun",
        ),
        (
            "retired entrypoint field",
            vec!["main"],
            Value::from("dist/index.js"),
            PluginDiscoveryIssueKind::InvalidToml,
            "main",
        ),
        (
            "unknown kind",
            vec!["kind"],
            Value::from("tool"),
            PluginDiscoveryIssueKind::InvalidManifest,
            "kind",
        ),
        (
            "non-integer resolver",
            vec!["resolver"],
            Value::from("1"),
            PluginDiscoveryIssueKind::InvalidToml,
            "resolver",
        ),
    ];

    for (label, path, replacement, expected_kind, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = agent_manifest();
        replace_path(&mut manifest, &path, replacement);
        write_manifest(temp_dir.path(), NAME, manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(
            (
                manager.installed_plugins().len(),
                manager.discovery_issues()[0].kind(),
                manager.discovery_issues()[0].field_path(),
            ),
            (0, expected_kind, Some(expected_field)),
            "{label}"
        );
    }
}

/// Verifies non-UTF-8 manifests fail safely instead of panicking.
#[test]
fn rejects_non_utf8_manifest() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_manifest(temp_dir.path(), NAME, &[0xff, 0xfe, 0xfd]);

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
    write_raw_manifest(
        temp_dir.path(),
        NAME,
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
        let mut manifest = agent_manifest();
        manifest["version"] = Value::from(version);
        write_manifest(temp_dir.path(), NAME, manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{version}");
        assert_eq!(
            (
                manager.discovery_issues()[0].kind(),
                manager.discovery_issues()[0].field_path()
            ),
            (PluginDiscoveryIssueKind::InvalidManifest, Some("version")),
            "{version}"
        );
    }
}

/// Verifies the plugin name and namespace grammar shared by every plugin kind.
#[test]
fn rejects_invalid_names_and_namespaces() {
    let long_segment = "a".repeat(70);
    let long_name = format!("{long_segment}.{long_segment}");
    let cases = [
        ("identifier", ""),
        ("identifier", "   "),
        ("identifier", "Ora.example"),
        ("identifier", "ora.skill_hub"),
        ("identifier", "ora.space.example"),
        ("identifier", "ora."),
        ("identifier", ".example"),
        ("identifier", "ora..example"),
        ("identifier", long_name.as_str()),
        ("namespace", ""),
        ("namespace", "Official"),
        ("namespace", "community"),
    ];
    for (field, value) in cases {
        let temp_dir = TempDir::new().unwrap();
        let mut manifest = agent_manifest();
        manifest[field] = Value::from(value);
        write_manifest(temp_dir.path(), NAME, manifest);

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{field}={value}");
        assert_eq!(
            (
                manager.discovery_issues()[0].kind(),
                manager.discovery_issues()[0].field_path()
            ),
            (PluginDiscoveryIssueKind::InvalidManifest, Some(field)),
            "{field}={value}"
        );
    }
}

/// Verifies the fixed `main.js` entrypoint must already be a regular file at the package root
/// when its package is discovered.
#[test]
fn rejects_missing_and_directory_entrypoints() {
    let missing_root = TempDir::new().unwrap();
    let package_root = write_manifest(missing_root.path(), NAME, agent_manifest());
    fs::remove_file(package_root.join("main.js")).unwrap();

    let missing = PluginManager::discover(missing_root.path());

    let directory_root = TempDir::new().unwrap();
    let package_root = write_manifest(directory_root.path(), NAME, agent_manifest());
    fs::remove_file(package_root.join("main.js")).unwrap();
    fs::create_dir(package_root.join("main.js")).unwrap();

    let directory = PluginManager::discover(directory_root.path());

    assert_eq!(
        (
            missing.installed_plugins(),
            missing.discovery_issues()[0].field_path(),
            directory.installed_plugins(),
            directory.discovery_issues()[0].field_path(),
        ),
        (&[][..], Some("main"), &[][..], Some("main"))
    );
}

/// Verifies canonical containment rejects an entrypoint symlink that targets outside its package.
#[test]
fn rejects_entrypoint_symlink_escape() {
    let temp_dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let package_root = write_manifest(temp_dir.path(), NAME, agent_manifest());
    let entrypoint = package_root.join("main.js");
    fs::remove_file(&entrypoint).unwrap();
    let outside_entrypoint = outside.path().join("outside.js");
    fs::write(&outside_entrypoint, "export {};\n").unwrap();
    if create_symlink(&outside_entrypoint, &entrypoint, SymlinkKind::File).is_err() {
        return;
    }

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues()[0].field_path(), Some("main"));
}

/// Verifies a symlinked package directory is neither discovered nor reported: the installer
/// only writes real directories, so a link is not an installed package.
#[test]
fn ignores_symlinked_package_directories() {
    let temp_dir = TempDir::new().unwrap();
    let checkout = TempDir::new().unwrap();
    fs::write(checkout.path().join("main.js"), "export {};\n").unwrap();
    fs::write(
        checkout.path().join("orax.toml"),
        toml::to_string(&agent_manifest()).unwrap(),
    )
    .unwrap();
    let installed = super::installed_root(temp_dir.path());
    fs::create_dir_all(&installed).unwrap();
    if create_symlink(
        checkout.path(),
        &installed.join(NAME),
        SymlinkKind::Directory,
    )
    .is_err()
    {
        return;
    }

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues(), &[]);
}

/// Verifies an agent package carrying a kind-specific section of another kind is rejected at
/// the section.
#[test]
fn rejects_agent_package_with_webview_section() {
    let temp_dir = TempDir::new().unwrap();
    let mut manifest = agent_manifest();
    replace_path(
        &mut manifest,
        &["webview"],
        toml::from_str(
            r#"
start_url = "https://example.com"
allowed_origins = ["https://example.com"]
"#,
        )
        .unwrap(),
    );
    write_manifest(temp_dir.path(), NAME, manifest);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues()[0].field_path(), Some("webview"));
}

/// Verifies a package written directly below `plugins/` (the pre-versioned layout) is neither
/// discovered nor reported: only the versioned installed root is a package location.
#[test]
fn ignores_legacy_install_layout() {
    let temp_dir = TempDir::new().unwrap();
    let legacy_root = temp_dir.path().join("plugins").join("legacy");
    fs::create_dir_all(&legacy_root).unwrap();
    fs::write(legacy_root.join("main.js"), "export {};\n").unwrap();
    fs::write(
        legacy_root.join("orax.toml"),
        toml::to_string(&agent_manifest()).unwrap(),
    )
    .unwrap();

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(manager.discovery_issues(), &[]);
}

/// Verifies only the highest installed version of a package is discovered.
#[test]
fn selects_highest_installed_version() {
    let temp_dir = TempDir::new().unwrap();
    let mut old = agent_manifest();
    old["version"] = Value::from("1.0.0");
    old["description"] = Value::from("Old");
    write_manifest(temp_dir.path(), NAME, old);
    let mut new = agent_manifest();
    new["version"] = Value::from("1.1.0");
    new["description"] = Value::from("New");
    let expected_root = write_manifest(temp_dir.path(), NAME, new);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(
        manager
            .installed_plugins()
            .iter()
            .map(|plugin| {
                (
                    plugin.version.to_string(),
                    plugin.description.clone(),
                    plugin.package_root.clone(),
                )
            })
            .collect::<Vec<_>>(),
        vec![("1.1.0".to_string(), "New".to_string(), expected_root)]
    );
}

/// Verifies a broken highest version is reported rather than silently replaced by an older
/// version the user no longer intended to run.
#[test]
fn does_not_fall_back_when_highest_version_is_broken() {
    let temp_dir = TempDir::new().unwrap();
    let mut old = agent_manifest();
    old["version"] = Value::from("1.0.0");
    write_manifest(temp_dir.path(), NAME, old);
    write_raw_manifest_version(temp_dir.path(), NAMESPACE, NAME, "1.1.0", b"{ invalid");

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![PluginDiscoveryIssueKind::InvalidToml]
    );
}

/// Verifies a version directory that is not SemVer is reported as an invalid install path.
#[test]
fn reports_non_semver_version_directories() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_manifest_version(
        temp_dir.path(),
        NAMESPACE,
        NAME,
        "latest",
        toml::to_string(&agent_manifest()).unwrap().as_bytes(),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| issue.kind())
            .collect::<Vec<_>>(),
        vec![PluginDiscoveryIssueKind::InvalidInstallPath]
    );
}

/// Verifies a manifest whose version disagrees with its directory is rejected on `version`.
#[test]
fn rejects_manifest_version_that_differs_from_directory() {
    let temp_dir = TempDir::new().unwrap();
    write_raw_manifest_version(
        temp_dir.path(),
        NAMESPACE,
        NAME,
        "1.1.0",
        toml::to_string(&agent_manifest()).unwrap().as_bytes(),
    );

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.installed_plugins(), &[]);
    assert_eq!(
        manager
            .discovery_issues()
            .iter()
            .map(|issue| (issue.kind(), issue.field_path()))
            .collect::<Vec<_>>(),
        vec![(PluginDiscoveryIssueKind::InvalidManifest, Some("version"))]
    );
}

/// Verifies root and manifest filesystem shapes are reported without panics and that stray
/// files below the installed root are ignored.
#[test]
fn reports_invalid_filesystem_shapes_and_ignores_stray_files() {
    let root_file = TempDir::new().unwrap();
    fs::create_dir_all(root_file.path().join("plugins")).unwrap();
    fs::write(root_file.path().join("plugins").join("installed"), "file").unwrap();
    let root_manager = PluginManager::discover(root_file.path());
    assert_eq!(
        root_manager.discovery_issues()[0].kind(),
        PluginDiscoveryIssueKind::RootUnreadable
    );

    let temp_dir = TempDir::new().unwrap();
    let namespace = super::installed_root(temp_dir.path()).join(NAMESPACE);
    let directory = namespace.join("ora.directory").join("1.0.0");
    let missing = namespace.join("ora.missing").join("1.0.0");
    fs::create_dir_all(directory.join("orax.toml")).unwrap();
    fs::create_dir_all(&missing).unwrap();
    fs::write(namespace.parent().unwrap().join("ignored.txt"), "ignored").unwrap();
    fs::write(namespace.join("ignored.txt"), "ignored").unwrap();
    fs::write(missing.join("notes.md"), "ignored").unwrap();
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

/// Creates the representative agent manifest.
pub(crate) fn agent_manifest() -> Value {
    toml::from_str(
        r#"
resolver = 1
identifier = "ora.claude-code"
namespace = "official"
kind = "agent"
version = "0.1.0"
description = "Claude Code agent"
homepage = "https://example.com/claude-code"
license = "Apache-2.0"

[dependencies]
ora = ">=0.1.0, <0.2.0"
"#,
    )
    .unwrap()
}

/// Creates the agent manifest under another plugin name.
fn named(name: &str) -> Value {
    let mut manifest = agent_manifest();
    manifest["identifier"] = Value::from(name);
    manifest
}

/// Writes one TOML manifest plus the fixed entrypoint into `installed/<directory>/` and returns
/// the package directory.
pub(crate) fn write_manifest(data_dir: &Path, directory: &str, manifest: Value) -> PathBuf {
    write_raw_manifest(
        data_dir,
        directory,
        toml::to_string(&manifest).unwrap().as_bytes(),
    )
}

/// Writes arbitrary bytes as one package manifest next to a valid entrypoint, under the versioned
/// `installed/<namespace>/<directory>/<version>/` layout.
///
/// The namespace and version directories are read leniently from the bytes so a deliberately
/// malformed manifest still lands in a layout discovery will visit; anything unparsable falls
/// back to a valid default so the test exercises the manifest failure it is about, not the path.
pub(crate) fn write_raw_manifest(data_dir: &Path, directory: &str, bytes: &[u8]) -> PathBuf {
    let toml = String::from_utf8_lossy(bytes);
    let namespace = manifest_string(&toml, "namespace")
        .filter(|namespace| !namespace.trim().is_empty())
        .unwrap_or(NAMESPACE);
    let version = manifest_string(&toml, "version")
        .filter(|version| Version::parse(version).is_ok())
        .unwrap_or("1.0.0");
    write_raw_manifest_version(data_dir, namespace, directory, version, bytes)
}

/// Writes arbitrary bytes as one package manifest into an explicit version directory.
fn write_raw_manifest_version(
    data_dir: &Path,
    namespace: &str,
    directory: &str,
    version: &str,
    bytes: &[u8],
) -> PathBuf {
    let package_root = super::installed_root(data_dir)
        .join(namespace)
        .join(directory)
        .join(version);
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("main.js"), "export {};\n").unwrap();
    fs::write(package_root.join("orax.toml"), bytes).unwrap();
    package_root
}

fn manifest_string<'a>(toml: &'a str, field: &str) -> Option<&'a str> {
    toml.lines().find_map(|line| {
        let value = line
            .strip_prefix(field)?
            .trim_start()
            .strip_prefix('=')?
            .trim();
        value.strip_prefix('"')?.strip_suffix('"')
    })
}

/// Verifies a package's `logo.svg` is discovered as trusted icon source text.
#[test]
fn discovers_package_logo() {
    let temp_dir = TempDir::new().unwrap();
    let logo = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="8"/></svg>"#;
    let package_root = write_manifest(temp_dir.path(), NAME, agent_manifest());
    fs::write(package_root.join("logo.svg"), logo).unwrap();

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins()[0].logo, Some(logo.to_string()));
}

/// Verifies a package without an icon is discovered cleanly instead of reporting a problem.
#[test]
fn discovers_package_without_a_logo() {
    let temp_dir = TempDir::new().unwrap();
    write_manifest(temp_dir.path(), NAME, agent_manifest());

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    assert_eq!(manager.installed_plugins()[0].logo, None);
}

/// Verifies an unsafe icon is reported and dropped while the plugin itself stays discovered.
#[test]
fn reports_an_unsafe_logo_without_hiding_the_plugin() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_manifest(temp_dir.path(), NAME, agent_manifest());
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

/// Replaces a nested TOML field, including array indices represented as decimal strings.
pub(crate) fn replace_path(value: &mut Value, path: &[&str], replacement: Value) {
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
        // `IndexMut` on a TOML table panics for absent keys, so insert through the table to
        // support adding fields the fixture does not declare.
        Err(_) => {
            current
                .as_table_mut()
                .unwrap()
                .insert(last.to_string(), replacement);
        }
    }
}

/// What kind of filesystem object a test symlink points at; Windows needs to know.
#[derive(Clone, Copy)]
pub(crate) enum SymlinkKind {
    File,
    Directory,
}

/// Creates a platform-native symlink when the test environment permits it.
#[cfg(unix)]
pub(crate) fn create_symlink(
    target: &Path,
    link: &Path,
    _kind: SymlinkKind,
) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Creates a Windows symlink when Developer Mode or privileges permit it.
#[cfg(windows)]
pub(crate) fn create_symlink(target: &Path, link: &Path, kind: SymlinkKind) -> std::io::Result<()> {
    match kind {
        SymlinkKind::File => std::os::windows::fs::symlink_file(target, link),
        SymlinkKind::Directory => std::os::windows::fs::symlink_dir(target, link),
    }
}
