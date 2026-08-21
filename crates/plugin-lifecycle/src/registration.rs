//! Checks a plugin's handshake registration against the contract its manifest kind implies.
//!
//! Validation runs once, right after launch, so a plugin that cannot serve its contract fails
//! visibly in the settings page instead of failing later in the middle of a user action.

use crate::ports::PluginRuntimeFailure;
use ora_plugin_manager::{InstalledSurfaceSource, PluginContribution};
use ora_plugin_runtime::PluginRegistration;

/// The request a remote-site ui plugin must serve so downloaded files can be handed to it.
pub const UI_DOWNLOAD_COMPLETED_METHOD: &str = "ui/downloadCompleted";
/// The request a panel ui plugin must serve so its page can reach the process.
pub const UI_REQUEST_METHOD: &str = "ui/request";

/// Rejects a registration that does not cover the contract implied by the manifest kind.
///
/// Agent plugins are accepted unconditionally here: the agent contract is verified by the agent
/// connection supervisor in the backend, which launches agent processes on its own, so checking
/// it again would duplicate the rule in two places that can drift.
pub fn validate_registration(
    contribution: &PluginContribution,
    registration: &PluginRegistration,
) -> Result<(), PluginRuntimeFailure> {
    match contribution {
        PluginContribution::Agent(_) => Ok(()),
        PluginContribution::Ui(ui) => {
            // Each source kind implies one required method; a plugin mixing both must serve both.
            let (mut has_remote_site, mut has_panel) = (false, false);
            for surface in &ui.surfaces {
                match &surface.source {
                    InstalledSurfaceSource::RemoteSite(_) => has_remote_site = true,
                    InstalledSurfaceSource::Panel(_) => has_panel = true,
                }
            }
            if has_remote_site && !registration.methods.contains(UI_DOWNLOAD_COMPLETED_METHOD) {
                return Err(PluginRuntimeFailure::new(format!(
                    "ui contract v1 requires method {UI_DOWNLOAD_COMPLETED_METHOD}"
                )));
            }
            if has_panel && !registration.methods.contains(UI_REQUEST_METHOD) {
                return Err(PluginRuntimeFailure::new(format!(
                    "ui contract v1 requires method {UI_REQUEST_METHOD}"
                )));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_registration;
    use crate::ports::PluginRuntimeFailure;
    use ora_plugin_manager::{
        InstalledPluginAgent, InstalledPluginUi, InstalledSurface, InstalledSurfaceSource,
        InstancePolicy, PanelSource, PluginContribution, RemoteSiteSource, SurfaceId,
        WebDataPolicy,
    };
    use ora_plugin_runtime::PluginRegistration;
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Builds one ui contribution with a single remote-site surface.
    fn remote_site_ui() -> PluginContribution {
        PluginContribution::Ui(InstalledPluginUi {
            contract_version: 1,
            surfaces: vec![InstalledSurface {
                id: SurfaceId::parse("market").expect("surface id"),
                title: "Market".to_string(),
                instance_policy: InstancePolicy::Singleton,
                source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                    entry_url: "https://example.com/".parse().expect("entry url"),
                    allow_hosts: Vec::new(),
                    allow_host_suffixes: Vec::new(),
                    web_data: WebDataPolicy::PersistentProfile,
                }),
            }],
        })
    }

    /// Builds one ui contribution with a single panel surface.
    fn panel_ui() -> PluginContribution {
        PluginContribution::Ui(InstalledPluginUi {
            contract_version: 1,
            surfaces: vec![InstalledSurface {
                id: SurfaceId::parse("counter").expect("surface id"),
                title: "Hello Panel".to_string(),
                instance_policy: InstancePolicy::Singleton,
                source: InstalledSurfaceSource::Panel(PanelSource {
                    asset_root: PathBuf::from("/plugins/hello-panel/ui"),
                    entry: PortableRelativePath::parse("index.html").expect("entry"),
                }),
            }],
        })
    }

    /// A panel ui plugin must serve `ui/request`; the download handler is not required of it.
    #[test]
    fn panel_ui_requires_request_only() {
        let with_request = PluginRegistration {
            methods: HashSet::from(["ui/request".to_string()]),
            emits: HashSet::from(["ui/push".to_string()]),
        };
        assert_eq!(
            (
                validate_registration(&panel_ui(), &PluginRegistration::default()),
                validate_registration(&panel_ui(), &with_request),
            ),
            (
                Err(PluginRuntimeFailure::new(
                    "ui contract v1 requires method ui/request"
                )),
                Ok(()),
            )
        );
    }

    /// A remote-site ui plugin without the download handler is rejected with the documented reason.
    #[test]
    fn remote_site_ui_requires_download_completed() {
        assert_eq!(
            validate_registration(&remote_site_ui(), &PluginRegistration::default()),
            Err(PluginRuntimeFailure::new(
                "ui contract v1 requires method ui/downloadCompleted"
            )),
        );
    }

    /// A remote-site ui plugin serving the download handler passes.
    #[test]
    fn remote_site_ui_with_download_completed_passes() {
        let registration = PluginRegistration {
            methods: HashSet::from(["ui/downloadCompleted".to_string()]),
            emits: HashSet::new(),
        };
        assert_eq!(
            validate_registration(&remote_site_ui(), &registration),
            Ok(())
        );
    }

    /// Agent contracts are verified by the agent supervisor, not here.
    #[test]
    fn agent_registrations_are_not_checked_here() {
        let contribution = PluginContribution::Agent(InstalledPluginAgent {
            display_name: "Agent".to_string(),
            contract_version: 1,
        });
        assert_eq!(
            validate_registration(&contribution, &PluginRegistration::default()),
            Ok(()),
        );
    }
}
