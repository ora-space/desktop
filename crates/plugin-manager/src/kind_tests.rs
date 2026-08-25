//! Discovery tests for the host-side policy of the `workbench` and `webview` kinds.

use super::tests::{agent_manifest, replace_path, write_manifest};
use super::{PluginContribution, PluginManager};
use ora_plugin_manifest::MethodName;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;
use toml::Value;

const NAME: &str = "ora.claude-code";

/// Builds a workbench manifest with the given methods list, or no `[workbench]` section.
fn workbench_manifest(methods: Option<&[&str]>) -> Value {
    let mut manifest = agent_manifest();
    manifest["kind"] = Value::from("workbench");
    if let Some(methods) = methods {
        let mut section = toml::Table::new();
        section.insert(
            "methods".to_string(),
            Value::Array(methods.iter().map(|m| Value::from(*m)).collect()),
        );
        manifest
            .as_table_mut()
            .unwrap()
            .insert("workbench".to_string(), Value::Table(section));
    }
    manifest
}

/// Builds a webview manifest from a literal `[webview]` table body.
fn webview_manifest(section: &str) -> Value {
    let mut manifest = agent_manifest();
    manifest["kind"] = Value::from("webview");
    replace_path(
        &mut manifest,
        &["webview"],
        toml::from_str(section).unwrap(),
    );
    manifest
}

/// Writes the page a workbench package must ship next to its manifest.
fn write_page(package_root: &std::path::Path) {
    fs::create_dir_all(package_root.join("assets")).unwrap();
    fs::write(
        package_root.join("assets").join("index.html"),
        "<html></html>\n",
    )
    .unwrap();
}

/// A workbench package resolves its canonical asset root and keeps declared methods in order;
/// omitting `[workbench]` yields a static page with no callable methods.
#[test]
fn discovers_workbench_package_with_and_without_methods() {
    let temp_dir = TempDir::new().unwrap();
    let package_root = write_manifest(
        temp_dir.path(),
        NAME,
        workbench_manifest(Some(&["counter/get", "counter/increment"])),
    );
    write_page(&package_root);
    let static_root = write_manifest(temp_dir.path(), "ora.static", {
        let mut manifest = workbench_manifest(None);
        manifest["name"] = Value::from("ora.static");
        manifest
    });
    write_page(&static_root);

    let manager = PluginManager::discover(temp_dir.path());

    assert_eq!(manager.discovery_issues(), &[]);
    let contributions: Vec<_> = manager
        .installed_plugins()
        .iter()
        .map(|plugin| match &plugin.contributes {
            PluginContribution::Workbench(descriptor) => (
                descriptor.asset_root.clone(),
                descriptor.declared_methods.clone(),
            ),
            PluginContribution::Agent(_)
            | PluginContribution::Webview(_)
            | PluginContribution::Skill(_) => {
                panic!("expected workbench contributions")
            }
        })
        .collect();
    assert_eq!(
        contributions,
        vec![
            (
                package_root.canonicalize().unwrap().join("assets"),
                vec![
                    MethodName::parse("counter/get").unwrap(),
                    MethodName::parse("counter/increment").unwrap(),
                ],
            ),
            (static_root.canonicalize().unwrap().join("assets"), vec![]),
        ]
    );
}

/// A workbench package must ship `main.js` and `assets/index.html`; either missing is reported.
#[test]
fn rejects_workbench_package_missing_entrypoint_or_page() {
    let without_page = TempDir::new().unwrap();
    write_manifest(without_page.path(), NAME, workbench_manifest(None));
    let without_main = TempDir::new().unwrap();
    let package_root = write_manifest(without_main.path(), NAME, workbench_manifest(None));
    write_page(&package_root);
    fs::remove_file(package_root.join("main.js")).unwrap();

    let page_issues = PluginManager::discover(without_page.path());
    let main_issues = PluginManager::discover(without_main.path());

    assert_eq!(
        (
            page_issues.installed_plugins().len(),
            page_issues.discovery_issues()[0].field_path(),
            main_issues.installed_plugins().len(),
            main_issues.discovery_issues()[0].field_path(),
        ),
        (0, Some("workbench"), 0, Some("main"))
    );
}

/// A webview package keeps its origins and rules, and must not ship an entrypoint.
#[test]
fn discovers_webview_package_and_refuses_an_entrypoint() {
    let section = r#"
start_url = "https://www.example.com/skills"
allowed_origins = ["https://www.example.com", "https://example.com"]
"#;
    let clean = TempDir::new().unwrap();
    let package_root = write_manifest(clean.path(), NAME, webview_manifest(section));
    fs::remove_file(package_root.join("main.js")).unwrap();
    let runnable = TempDir::new().unwrap();
    write_manifest(runnable.path(), NAME, webview_manifest(section));

    let clean_manager = PluginManager::discover(clean.path());
    let runnable_manager = PluginManager::discover(runnable.path());

    let origins = match &clean_manager.installed_plugins()[0].contributes {
        PluginContribution::Webview(descriptor) => descriptor
            .allowed_origins
            .iter()
            .map(|origin| origin.as_str().to_string())
            .collect::<Vec<_>>(),
        PluginContribution::Agent(_)
        | PluginContribution::Workbench(_)
        | PluginContribution::Skill(_) => {
            panic!("expected a webview contribution")
        }
    };
    assert_eq!(
        (
            clean_manager.discovery_issues(),
            origins,
            runnable_manager.installed_plugins().len(),
            runnable_manager.discovery_issues()[0].field_path(),
        ),
        (
            &[][..],
            vec![
                "https://www.example.com".to_string(),
                "https://example.com".to_string()
            ],
            0,
            Some("kind"),
        )
    );
}

/// Cross-value webview rules: duplicate origins, an uncovered start URL, a rule on an origin
/// outside the set, and a rule shadowed by an earlier one each report their field.
#[test]
fn rejects_webview_cross_value_violations_with_field_paths() {
    let cases = [
        (
            r#"
start_url = "https://a.example"
allowed_origins = ["https://a.example", "https://A.example"]
"#,
            "webview.allowed_origins[1]",
        ),
        (
            r#"
start_url = "https://b.example"
allowed_origins = ["https://a.example"]
"#,
            "webview.start_url",
        ),
        (
            r#"
start_url = "https://a.example"
allowed_origins = ["https://a.example"]
[[downloads.rules]]
page = { origin = "https://b.example", path_prefix = "/" }
action = { reject = true }
"#,
            "webview.downloads.rules[0].page.origin",
        ),
        (
            r#"
start_url = "https://a.example"
allowed_origins = ["https://a.example"]
[[downloads.rules]]
page = { origin = "https://a.example", path_prefix = "/files/" }
action = { prompt = ["save_as"] }
[[downloads.rules]]
page = { origin = "https://a.example", path_prefix = "/files/skills/" }
action = { auto = "import_skill" }
"#,
            "webview.downloads.rules[1]",
        ),
    ];
    for (section, expected_field) in cases {
        let temp_dir = TempDir::new().unwrap();
        let package_root = write_manifest(temp_dir.path(), NAME, webview_manifest(section));
        fs::remove_file(package_root.join("main.js")).unwrap();

        let manager = PluginManager::discover(temp_dir.path());

        assert_eq!(manager.installed_plugins(), &[], "{section}");
        assert_eq!(
            manager.discovery_issues()[0].field_path(),
            Some(expected_field),
            "{section}"
        );
    }
}
