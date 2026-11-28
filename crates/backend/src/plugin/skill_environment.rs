//! Freezes Skill configuration at backend startup for both Agent process launch paths.

use ora_plugin_config::{
    ConfigurationService, SettingValue, SkillEnvironmentError, resolve_skill_environment,
    skill_environment_key,
};
use ora_plugin_lifecycle::ChildProcessEnvironmentProvider;
use ora_plugin_manager::{PluginContribution, PluginManager};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

/// Shares one immutable startup snapshot; deliberately has no Debug implementation for secrets.
#[derive(Clone, Default)]
pub(crate) struct SkillEnvironment {
    values: Arc<BTreeMap<String, String>>,
    /// The `secret`-flagged `String` setting values delivered to Agent processes, kept as a
    /// separate subset for log redaction (ADR D4).
    secrets: Arc<BTreeSet<String>>,
}

impl SkillEnvironment {
    /// Reads only valid installed Skill packages and never exposes rejected configuration values.
    pub(crate) fn capture(home: &Path, configuration: &ConfigurationService) -> Self {
        let mut candidates = BTreeMap::new();
        let mut names: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for plugin in PluginManager::discover(home).installed_plugins() {
            if !matches!(plugin.contributes, PluginContribution::Skill(_)) {
                continue;
            }
            let id = plugin.id.canonical();
            let details = match configuration.get(&id, &plugin.package_root) {
                Ok(Some(details)) => details,
                Ok(None) => continue,
                Err(_) => {
                    // Configuration errors may contain parser input; never log the source error.
                    ora_logging::ora_warn!(plugin_id = %id, "Skill environment omitted: configuration could not be read");
                    continue;
                }
            };
            // Declaration identity must not change when users fill, clear or corrupt values.
            for setting in &details.settings {
                names
                    .entry(skill_environment_key(&id, &setting.declaration.id).to_ascii_uppercase())
                    .or_default()
                    .push((id.clone(), setting.declaration.id.clone()));
            }
            let values = match resolve_skill_environment(&id, &details) {
                Ok(values) => Some(values),
                Err(SkillEnvironmentError::NameCollision) => None,
                Err(SkillEnvironmentError::InvalidConfiguration) => {
                    ora_logging::ora_warn!(plugin_id = %id, "Skill environment omitted: configuration is incomplete or incompatible with process environment values");
                    None
                }
            };
            if let Some(values) = values {
                // The secret subset follows the same atomic resolution: a package that was not
                // injected does not register its secrets either (ADR D4).
                let secrets: BTreeSet<String> = details
                    .settings
                    .iter()
                    .filter(|setting| setting.declaration.secret)
                    .filter_map(|setting| match &setting.effective_value {
                        Some(SettingValue::String(value)) => Some(value.clone()),
                        _ => None,
                    })
                    .collect();
                candidates.insert(id, (values, secrets));
            }
        }
        let mut rejected = BTreeSet::new();
        for owners in names.values().filter(|owners| owners.len() > 1) {
            for (plugin_id, field_id) in owners {
                rejected.insert(plugin_id.clone());
                ora_logging::ora_warn!(plugin_id = %plugin_id, field_id = %field_id, "Skill environment omitted: environment name collision");
            }
        }
        let mut values = BTreeMap::new();
        let mut secrets = BTreeSet::new();
        for (id, (package_values, package_secrets)) in candidates {
            if rejected.contains(&id) {
                continue;
            }
            values.extend(package_values);
            secrets.extend(package_secrets);
        }
        Self {
            values: Arc::new(values),
            secrets: Arc::new(secrets),
        }
    }
}

impl ChildProcessEnvironmentProvider for SkillEnvironment {
    /// Gives host-spawned Agent children the same snapshot as directly spawned children.
    fn environment(
        &self,
        _plugin_id: &str,
        _workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        Ok((*self.values).clone())
    }

    /// Gives every Agent plugin the desktop-wide snapshot, independent of workspace selection.
    fn plugin_environment(&self, _plugin_id: &str) -> BTreeMap<String, String> {
        (*self.values).clone()
    }

    /// Returns the desktop-wide Skill secret snapshot for redaction; the receiving Agent plugin
    /// does not change the set (ADR D4).
    fn secret_values(&self, _plugin_id: &str) -> Vec<String> {
        self.secrets.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_plugin_config::{SettingValue, skill_environment_key};
    use pretty_assertions::assert_eq;

    #[derive(Clone, Default)]
    struct Diagnostics(Arc<std::sync::Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Diagnostics {
        /// Records every structured field to detect accidental value disclosure.
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Fields(String);
            impl tracing::field::Visit for Fields {
                /// Retains field names and values for the redaction assertion.
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0.push_str(&format!("{field}={value:?};"));
                }
            }
            let mut fields = Fields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }

    /// Uses real package discovery and config parsing for each collision family in both orders.
    #[test]
    fn collisions_omit_entire_packages_and_diagnostics_never_include_values() {
        ora_logging::initialize_test_clock();
        for pair in [
            [("a/b-c", "token"), ("a/b.c", "token")],
            [("a-b/c", "token"), ("a/b-c", "token")],
            [("a/b", "tokenA"), ("a/b", "tokena")],
        ] {
            for reverse in [false, true] {
                let diagnostics = Diagnostics::default();
                ora_logging::with_recorded_trace_logging(diagnostics.clone(), || {
                    let root = tempfile::tempdir().unwrap();
                    let mut plugins =
                        BTreeMap::<&str, serde_json::Map<String, serde_json::Value>>::new();
                    let mut declarations = vec![pair[0], pair[1], ("other/safe-token", "token")];
                    if reverse {
                        declarations.reverse();
                    }
                    for (id, field) in declarations {
                        let settings = plugins.entry(id).or_default();
                        for field in [field, "unrelated"] {
                            settings.insert(field.into(), serde_json::json!({"type":"string","title":"Value","description":"Value","default":"test-only-sensitive-value"}));
                        }
                    }
                    // An invalid declaration has no valid field set and must not poison another owner.
                    let invalid =
                        serde_json::json!({"type":"unknown","title":"Value","description":"Value"});
                    plugins
                        .entry("other-safe/token")
                        .or_default()
                        .insert("token".into(), invalid);
                    for (id, settings) in plugins {
                        let (namespace, identifier) = id.split_once('/').unwrap();
                        let package = root
                            .path()
                            .join("plugins/installed")
                            .join(namespace)
                            .join(identifier)
                            .join("1.0.0");
                        std::fs::create_dir_all(package.join("assets/demo")).unwrap();
                        std::fs::write(package.join("orax.toml"), format!("identifier = \"{identifier}\"\nnamespace = \"{namespace}\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Demo\"\n")).unwrap();
                        std::fs::write(
                            package.join("assets/demo/SKILL.md"),
                            "---\nname: demo\ndescription: Demo\n---\n",
                        )
                        .unwrap();
                        std::fs::write(
                            package.join("assets/config.json"),
                            serde_json::json!({"schemaVersion":1,"settings":settings}).to_string(),
                        )
                        .unwrap();
                    }
                    let snapshot = SkillEnvironment::capture(
                        root.path(),
                        &ConfigurationService::new(root.path()),
                    );
                    assert_eq!(
                        snapshot.plugin_environment("agent"),
                        BTreeMap::from([
                            (
                                skill_environment_key("other/safe-token", "token"),
                                "test-only-sensitive-value".into()
                            ),
                            (
                                skill_environment_key("other/safe-token", "unrelated"),
                                "test-only-sensitive-value".into()
                            ),
                        ])
                    );
                });
                let messages = diagnostics.0.lock().unwrap().join("\n");
                assert!(
                    messages.contains("environment name collision"),
                    "{messages}"
                );
                assert!(messages.contains("plugin_id="), "{messages}");
                assert!(messages.contains("field_id="), "{messages}");
                assert!(
                    !messages.contains("test-only-sensitive-value"),
                    "{messages}"
                );
            }
        }
    }

    /// Value availability never changes collision ownership; invalid declarations are excluded.
    #[test]
    fn declaration_names_participate_before_value_eligibility() {
        ora_logging::initialize_test_clock();
        ora_logging::with_trace_logging(|| {
            for state in [
                "optional",
                "required",
                "nul",
                "corrupt",
                "invalid-value",
                "invalid-declaration",
            ] {
                let root = tempfile::tempdir().unwrap();
                for id in ["a/b-c", "a/b.c", "other/safe"] {
                    let (namespace, identifier) = id.split_once('/').unwrap();
                    let package = root
                        .path()
                        .join("plugins/installed")
                        .join(namespace)
                        .join(identifier)
                        .join("1.0.0");
                    std::fs::create_dir_all(package.join("assets/demo")).unwrap();
                    std::fs::write(package.join("orax.toml"), format!("identifier = \"{identifier}\"\nnamespace = \"{namespace}\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Demo\"\n")).unwrap();
                    std::fs::write(
                        package.join("assets/demo/SKILL.md"),
                        "---\nname: demo\ndescription: Demo\n---\n",
                    )
                    .unwrap();
                    let mut token = serde_json::json!({"type":"string","title":"Token","description":"Token","default":"valid"});
                    if id == "a/b-c" {
                        match state {
                            "optional" => {
                                token.as_object_mut().unwrap().remove("default");
                            }
                            "required" => {
                                token.as_object_mut().unwrap().remove("default");
                                token["required"] = true.into();
                            }
                            "nul" => token["default"] = "invalid\0value".into(),
                            "invalid-declaration" => token["type"] = "unknown".into(),
                            _ => {}
                        }
                        if matches!(state, "corrupt" | "invalid-value") {
                            let data = root
                                .path()
                                .join("plugins/data")
                                .join(namespace)
                                .join(identifier);
                            std::fs::create_dir_all(&data).unwrap();
                            std::fs::write(
                                data.join("store.json"),
                                if state == "corrupt" {
                                    "{broken"
                                } else {
                                    r#"{"schemaVersion":1,"revision":1,"values":{"token":42}}"#
                                },
                            )
                            .unwrap();
                        }
                    }
                    std::fs::write(package.join("assets/config.json"), serde_json::json!({"schemaVersion":1,"settings":{
                        "token":token,
                        (if id == "a/b-c" { "leftover" } else { "unrelated" }):{"type":"string","title":"Other","description":"Other","default":"valid"}
                    }}).to_string()).unwrap();
                }
                let snapshot =
                    SkillEnvironment::capture(root.path(), &ConfigurationService::new(root.path()));
                let mut expected = BTreeMap::from([
                    (skill_environment_key("other/safe", "token"), "valid".into()),
                    (
                        skill_environment_key("other/safe", "unrelated"),
                        "valid".into(),
                    ),
                ]);
                if state == "invalid-declaration" {
                    expected.insert(skill_environment_key("a/b.c", "token"), "valid".into());
                    expected.insert(skill_environment_key("a/b.c", "unrelated"), "valid".into());
                }
                assert_eq!(snapshot.plugin_environment("agent"), expected, "{state}");
            }
        });
    }

    /// Real discovery and storage changes do not mutate the already captured process environment.
    #[test]
    fn startup_snapshot_changes_only_after_recapture() {
        ora_logging::initialize_test_clock();
        ora_logging::with_trace_logging(|| {
            let root = tempfile::tempdir().unwrap();
            let package = root.path().join("plugins/installed/official/demo/1.0.0");
            std::fs::create_dir_all(package.join("assets/demo")).unwrap();
            std::fs::write(package.join("orax.toml"), "identifier = \"demo\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Demo\"\n").unwrap();
            std::fs::write(
                package.join("assets/demo/SKILL.md"),
                "---\nname: demo\ndescription: Demo\n---\n",
            )
            .unwrap();
            std::fs::write(package.join("assets/config.json"), r#"{"schemaVersion":1,"settings":{"token":{"type":"string","title":"Token","description":"Token","required":true,"secret":true}}}"#).unwrap();
            let configuration = ConfigurationService::new(root.path());
            assert!(
                SkillEnvironment::capture(root.path(), &configuration)
                    .values
                    .is_empty()
            );
            let details = configuration
                .get("official/demo", &package)
                .unwrap()
                .unwrap();
            configuration
                .save_preserving(
                    "official/demo",
                    &package,
                    details.revision,
                    &details.declaration.fingerprint,
                    BTreeMap::from([(
                        "token".into(),
                        SettingValue::String("test-only-secret".into()),
                    )]),
                    &Default::default(),
                )
                .unwrap();
            let captured = SkillEnvironment::capture(root.path(), &configuration);
            let expected = BTreeMap::from([(
                skill_environment_key("official/demo", "token"),
                "test-only-secret".into(),
            )]);
            assert_eq!(captured.plugin_environment("agent"), expected);
            assert_eq!(
                captured
                    .environment("agent", Path::new("workspace"))
                    .unwrap(),
                expected
            );
            assert_eq!(
                captured.secret_values("agent"),
                vec!["test-only-secret".to_string()],
                "only the secret-flagged String value is registered for redaction",
            );
            let current = configuration
                .get("official/demo", &package)
                .unwrap()
                .unwrap();
            configuration
                .reset_all(
                    "official/demo",
                    &package,
                    current.revision,
                    &current.declaration.fingerprint,
                )
                .unwrap();
            assert_eq!(captured.plugin_environment("agent"), expected);
            assert!(
                SkillEnvironment::capture(root.path(), &configuration)
                    .values
                    .is_empty()
            );
            std::fs::remove_dir_all(&package).unwrap();
            assert_eq!(captured.plugin_environment("agent"), expected);
            assert!(
                SkillEnvironment::capture(root.path(), &configuration)
                    .values
                    .is_empty()
            );
        });
    }
}
