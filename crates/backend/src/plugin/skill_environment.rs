//! Freezes Skill configuration at backend startup for both Agent process launch paths.

use ora_plugin_config::{ConfigurationService, resolve_skill_environment};
use ora_plugin_lifecycle::ChildProcessEnvironmentProvider;
use ora_plugin_manager::{PluginContribution, PluginManager};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// Shares one immutable startup snapshot; deliberately has no Debug implementation for secrets.
#[derive(Clone, Default)]
pub(crate) struct SkillEnvironment(Arc<BTreeMap<String, String>>);

impl SkillEnvironment {
    /// Reads only valid installed Skill packages and never exposes rejected configuration values.
    pub(crate) fn capture(home: &Path, configuration: &ConfigurationService) -> Self {
        let mut environment = BTreeMap::new();
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
            if let Some(values) = resolve_skill_environment(&id, &details) {
                environment.extend(values);
            } else {
                ora_logging::ora_warn!(plugin_id = %id, "Skill environment omitted: configuration is incomplete or incompatible with process environment values");
            }
        }
        Self(Arc::new(environment))
    }
}

impl ChildProcessEnvironmentProvider for SkillEnvironment {
    /// Gives host-spawned Agent children the same snapshot as directly spawned children.
    fn environment(
        &self,
        _plugin_id: &str,
        _workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        Ok((*self.0).clone())
    }

    /// Gives every Agent plugin the desktop-wide snapshot, independent of workspace selection.
    fn plugin_environment(&self, _plugin_id: &str) -> BTreeMap<String, String> {
        (*self.0).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_plugin_config::{SettingValue, skill_environment_key};
    use pretty_assertions::assert_eq;

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
                    .0
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
                    .0
                    .is_empty()
            );
            std::fs::remove_dir_all(&package).unwrap();
            assert_eq!(captured.plugin_environment("agent"), expected);
            assert!(
                SkillEnvironment::capture(root.path(), &configuration)
                    .0
                    .is_empty()
            );
        });
    }
}
