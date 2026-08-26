//! Checks a plugin's handshake registration against the contract its manifest kind implies.
//!
//! Validation runs once, right after launch, so a plugin that cannot serve its contract fails
//! visibly in the settings page instead of failing later in the middle of a user action.

use crate::ports::PluginRuntimeFailure;
use ora_plugin_manager::PluginContribution;
use ora_plugin_manifest::MethodName;
use ora_plugin_runtime::PluginRegistration;

/// Rejects a registration that does not fit the contract implied by the manifest kind.
///
/// Agent plugins are accepted unconditionally here: the agent contract is verified by the agent
/// connection supervisor in the backend, so checking it again would duplicate the rule in two
/// places that can drift. A workbench plugin may register any well-formed method names (the
/// page-visible subset is decided per surface as the intersection with the manifest), but must
/// not declare `emits`: v1 has no plugin-to-page channel, and the runtime kills a process that
/// sends an undeclared notification, so an unexpected declaration is better refused at the
/// handshake than discovered as a silent no-op.
pub fn validate_registration(
    contribution: &PluginContribution,
    registration: &PluginRegistration,
) -> Result<(), PluginRuntimeFailure> {
    match contribution {
        PluginContribution::Agent(_) => Ok(()),
        PluginContribution::Workbench(_) => {
            if !registration.effect_surfaces.is_empty() {
                return Err(PluginRuntimeFailure::new(
                    "workbench contract v1 does not accept Effect surface declarations",
                ));
            }
            if let Some(emit) = registration.emits.iter().next() {
                return Err(PluginRuntimeFailure::new(format!(
                    "workbench contract v1 does not accept emitted notifications (found {emit})"
                )));
            }
            if let Some(method) = registration
                .methods
                .iter()
                .find(|method| MethodName::parse(method).is_err())
            {
                return Err(PluginRuntimeFailure::new(format!(
                    "workbench contract v1 rejects method name {method}"
                )));
            }
            Ok(())
        }
        PluginContribution::Webview(_) => Err(PluginRuntimeFailure::new(
            "webview plugins have no process and cannot register",
        )),
        PluginContribution::Skill(_) => Err(PluginRuntimeFailure::new(
            "skill plugins have no process and cannot register",
        )),
        PluginContribution::Mcp(_) => Err(PluginRuntimeFailure::new(
            "mcp plugins have no process and cannot register",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_registration;
    use crate::ports::PluginRuntimeFailure;
    use ora_plugin_manager::{
        InstalledPluginAgent, InstalledWorkbenchDescriptor, PluginContribution,
    };
    use ora_plugin_runtime::PluginRegistration;
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Builds one workbench contribution declaring `counter/get` to the page.
    fn workbench() -> PluginContribution {
        PluginContribution::Workbench(InstalledWorkbenchDescriptor {
            entrypoint: PortableRelativePath::parse("main.js").expect("entrypoint"),
            asset_root: PathBuf::from("/plugins/example/assets"),
            page_entry: PortableRelativePath::parse("index.html").expect("page entry"),
            declared_methods: vec![
                ora_plugin_manifest::MethodName::parse("counter/get").expect("method"),
            ],
        })
    }

    /// A workbench plugin may register more or fewer methods than the manifest declares, but
    /// never an emitted notification or a malformed method name.
    #[test]
    fn workbench_registrations_reject_emits_and_bad_names() {
        let with_emit = PluginRegistration {
            methods: HashSet::from(["counter/get".to_string()]),
            emits: HashSet::from(["counter/tick".to_string()]),
            effect_surfaces: Vec::new(),
        };
        let bad_name = PluginRegistration {
            methods: HashSet::from(["Counter.Get".to_string()]),
            emits: HashSet::new(),
            effect_surfaces: Vec::new(),
        };
        let superset = PluginRegistration {
            methods: HashSet::from(["counter/get".to_string(), "internal/reset".to_string()]),
            emits: HashSet::new(),
            effect_surfaces: Vec::new(),
        };
        assert_eq!(
            (
                validate_registration(&workbench(), &with_emit),
                validate_registration(&workbench(), &bad_name),
                validate_registration(&workbench(), &superset),
                validate_registration(&workbench(), &PluginRegistration::default()),
            ),
            (
                Err(PluginRuntimeFailure::new(
                    "workbench contract v1 does not accept emitted notifications (found counter/tick)"
                )),
                Err(PluginRuntimeFailure::new(
                    "workbench contract v1 rejects method name Counter.Get"
                )),
                Ok(()),
                Ok(()),
            )
        );
    }

    /// Runtime Effect consumers are Agent plugins; a page process cannot own that lifecycle.
    #[test]
    fn workbench_registrations_reject_effect_surfaces() {
        let registration = PluginRegistration {
            effect_surfaces: vec![ora_plugin_runtime::PluginEffectSurface {
                workspace_relative_path: ".agents/skills".to_string(),
                materialization_format: "skill_directory.v1".to_string(),
                coordination: ora_plugin_runtime::PluginEffectCoordination::Uninterrupted,
            }],
            ..PluginRegistration::default()
        };
        assert_eq!(
            validate_registration(&workbench(), &registration),
            Err(PluginRuntimeFailure::new(
                "workbench contract v1 does not accept Effect surface declarations",
            )),
        );
    }

    /// Agent contracts are verified by the agent supervisor, not here.
    #[test]
    fn agent_registrations_are_not_checked_here() {
        let contribution = PluginContribution::Agent(InstalledPluginAgent {
            display_name: "Agent".to_string(),
            entrypoint: PortableRelativePath::parse("main.js").expect("entrypoint"),
        });
        assert_eq!(
            validate_registration(&contribution, &PluginRegistration::default()),
            Ok(()),
        );
    }

    /// Static skill plugins have no runtime handshake.
    #[test]
    fn skill_plugins_cannot_register() {
        assert_eq!(
            validate_registration(
                &PluginContribution::Skill(Default::default()),
                &PluginRegistration::default(),
            ),
            Err(PluginRuntimeFailure::new(
                "skill plugins have no process and cannot register",
            )),
        );
    }

    /// MCP plugins are configuration-only and have no runtime handshake.
    #[test]
    fn mcp_plugins_cannot_register() {
        use ora_plugin_config::{CompiledConfigurationFile, compile_configuration_file};
        use ora_plugin_manager::InstalledMcpDescriptor;

        let CompiledConfigurationFile::Mcp(configuration) = compile_configuration_file(
            br#"{ "schemaVersion": 1, "transport": { "type": "http", "url": "https://mcp.example.com/v1" } }"#,
        )
        .expect("compile fixture") else {
            panic!("expected the MCP shape");
        };
        let contribution = PluginContribution::Mcp(InstalledMcpDescriptor { configuration });
        assert_eq!(
            validate_registration(&contribution, &PluginRegistration::default()),
            Err(PluginRuntimeFailure::new(
                "mcp plugins have no process and cannot register",
            )),
        );
    }
}
