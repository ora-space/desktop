use crate::PluginLifecycleError;
use ora_application::PluginStateRepository;
use ora_contracts::{
    InstalledPlugin, InstalledPluginContribution, PluginConfigurationCompleteness,
    PluginConfigurationSummary, PluginInstallationValidity, PluginRuntimeStatus,
};
use ora_domain::{PluginEnabledState, PluginId};
use ora_plugin_config::{ConfigurationCompleteness, ConfigurationService, ConfigurationSummary};
use ora_plugin_manager::InstalledPlugin as DiscoveredPlugin;
use ora_plugin_manager::{PluginConfigurationDeclarationValidity, PluginContribution};
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::watch;

/// Observable copy of one plugin's managed state; `None` means the plugin is not installed.
pub(super) type ManagedStateWatch<Runtime> = watch::Sender<Option<ManagedPluginState<Runtime>>>;

/// Holds the filesystem snapshot and process-scoped lifecycle state as one atomic view.
///
/// `managed_by_id` is mutated only through `set_managed`, `remove_managed`, and
/// `replace_managed` so every transition is mirrored into the per-plugin watch channel that
/// `ensure_running` waits on; a direct map write would leave waiters stuck on a stale snapshot.
pub(super) struct LifecycleState<Runtime> {
    pub(super) installed: Vec<DiscoveredPlugin>,
    managed_by_id: BTreeMap<PluginId, ManagedPluginState<Runtime>>,
    status_by_id: BTreeMap<PluginId, ManagedStateWatch<Runtime>>,
    pub(super) next_attempt: u64,
}

impl<Runtime: Clone> LifecycleState<Runtime> {
    /// Builds the initial state and one watch channel per managed plugin.
    pub(super) fn new(
        installed: Vec<DiscoveredPlugin>,
        managed_by_id: BTreeMap<PluginId, ManagedPluginState<Runtime>>,
    ) -> Self {
        let status_by_id = managed_by_id
            .iter()
            .map(|(plugin_id, managed)| {
                (plugin_id.clone(), watch::Sender::new(Some(managed.clone())))
            })
            .collect();
        Self {
            installed,
            managed_by_id,
            status_by_id,
            next_attempt: 1,
        }
    }

    /// Returns the current managed state of one plugin.
    pub(super) fn managed(&self, plugin_id: &PluginId) -> Option<&ManagedPluginState<Runtime>> {
        self.managed_by_id.get(plugin_id)
    }

    /// Records one transition and wakes every waiter subscribed to that plugin.
    pub(super) fn set_managed(
        &mut self,
        plugin_id: &PluginId,
        managed: ManagedPluginState<Runtime>,
    ) {
        self.status_by_id
            .entry(plugin_id.clone())
            .or_insert_with(|| watch::Sender::new(None))
            .send_replace(Some(managed.clone()));
        self.managed_by_id.insert(plugin_id.clone(), managed);
    }

    /// Forgets one plugin; dropping its sender tells waiters the plugin no longer exists.
    pub(super) fn remove_managed(
        &mut self,
        plugin_id: &PluginId,
    ) -> Option<ManagedPluginState<Runtime>> {
        self.status_by_id.remove(plugin_id);
        self.managed_by_id.remove(plugin_id)
    }

    /// Replaces the whole managed map after a scan, reconciling watch channels plugin by plugin.
    pub(super) fn replace_managed(
        &mut self,
        managed_by_id: BTreeMap<PluginId, ManagedPluginState<Runtime>>,
    ) -> BTreeMap<PluginId, ManagedPluginState<Runtime>> {
        let removed = self
            .managed_by_id
            .keys()
            .filter(|plugin_id| !managed_by_id.contains_key(*plugin_id))
            .cloned()
            .collect::<Vec<_>>();
        for plugin_id in removed {
            self.status_by_id.remove(&plugin_id);
        }
        for (plugin_id, managed) in &managed_by_id {
            self.status_by_id
                .entry(plugin_id.clone())
                .or_insert_with(|| watch::Sender::new(None))
                .send_replace(Some(managed.clone()));
        }
        std::mem::replace(&mut self.managed_by_id, managed_by_id)
    }

    /// Subscribes to one plugin's transitions, starting from its current snapshot.
    ///
    /// A plugin that is not managed yields `None` immediately rather than an error so callers
    /// can treat "uninstalled" uniformly whether it happened before or during their wait.
    pub(super) fn subscribe(
        &mut self,
        plugin_id: &PluginId,
    ) -> watch::Receiver<Option<ManagedPluginState<Runtime>>> {
        match self.status_by_id.get(plugin_id) {
            Some(sender) => sender.subscribe(),
            None => watch::Sender::new(None).subscribe(),
        }
    }
}

/// Makes the illegal combination of a disabled plugin with a live runtime unrepresentable.
#[derive(Clone)]
pub(super) enum ManagedPluginState<Runtime> {
    Disabled,
    Enabled(EnabledRuntime<Runtime>),
}

/// Represents every process-scoped state available only to an enabled plugin.
#[derive(Clone)]
pub(super) enum EnabledRuntime<Runtime> {
    Stopped,
    Starting { attempt: u64 },
    Running { attempt: u64, runtime: Runtime },
    Failed { reason: String },
}

/// Removes orphan rows and builds stopped runtime state for every discovered package.
pub(super) fn reconcile_persisted_state<Repository, Runtime>(
    repository: &Repository,
    installed: &[DiscoveredPlugin],
) -> Result<BTreeMap<PluginId, ManagedPluginState<Runtime>>, PluginLifecycleError>
where
    Repository: PluginStateRepository,
{
    let installed_ids = installed
        .iter()
        .map(|plugin| plugin.id.clone())
        .collect::<BTreeSet<_>>();
    let mut enabled_by_id = BTreeMap::new();
    for state in repository
        .list_plugin_states()
        .map_err(PluginLifecycleError::Repository)?
    {
        if installed_ids.contains(&state.plugin_id) {
            enabled_by_id.insert(state.plugin_id, state.enabled);
        } else {
            repository
                .delete_plugin_state(&state.plugin_id)
                .map_err(PluginLifecycleError::Repository)?;
        }
    }

    Ok(installed_ids
        .into_iter()
        .map(|plugin_id| {
            let managed = match enabled_by_id
                .get(&plugin_id)
                .copied()
                .unwrap_or(PluginEnabledState::Disabled)
            {
                PluginEnabledState::Enabled => ManagedPluginState::Enabled(EnabledRuntime::Stopped),
                PluginEnabledState::Disabled => ManagedPluginState::Disabled,
            };
            (plugin_id, managed)
        })
        .collect())
}

/// Maps package identity plus the illegal-state-free internal lifecycle enum to contracts.
pub(super) fn discovered_plugin_contract<Runtime>(
    plugin: &DiscoveredPlugin,
    managed: &ManagedPluginState<Runtime>,
    configuration: &ConfigurationService,
) -> InstalledPlugin {
    let (enabled, runtime) = match managed {
        ManagedPluginState::Disabled => (false, PluginRuntimeStatus::Stopped),
        ManagedPluginState::Enabled(EnabledRuntime::Stopped) => {
            (true, PluginRuntimeStatus::Stopped)
        }
        ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }) => {
            (true, PluginRuntimeStatus::Starting)
        }
        ManagedPluginState::Enabled(EnabledRuntime::Running { .. }) => {
            (true, PluginRuntimeStatus::Running)
        }
        ManagedPluginState::Enabled(EnabledRuntime::Failed { reason }) => (
            true,
            PluginRuntimeStatus::Failed {
                failure_reason: reason.clone(),
            },
        ),
    };

    // Project the validated contribution onto the wire enum; the frontend only renders an entry,
    // so asset paths, origin allow lists, and download rules never cross the boundary.
    let contribution = match &plugin.contributes {
        PluginContribution::Agent(agent) => InstalledPluginContribution::Agent {
            agent_display_name: agent.display_name.clone(),
        },
        PluginContribution::Workbench(_) => InstalledPluginContribution::Workbench {
            title: plugin.display_name.clone(),
        },
        PluginContribution::Webview(webview) => InstalledPluginContribution::Webview {
            title: plugin.display_name.clone(),
            start_url: webview.start_url.to_string(),
        },
        PluginContribution::Skill(_) => InstalledPluginContribution::Skill,
        PluginContribution::Mcp(_) => InstalledPluginContribution::Mcp,
    };

    let installation_validity = match &plugin.configuration_declaration {
        PluginConfigurationDeclarationValidity::Invalid { .. } => {
            PluginInstallationValidity::InvalidDeclaration {
                error_code: "plugin_configuration_declaration_invalid".to_string(),
            }
        }
        PluginConfigurationDeclarationValidity::NotDeclared
        | PluginConfigurationDeclarationValidity::Valid => PluginInstallationValidity::Valid,
    };
    let configuration = match configuration.summary(&plugin.id.canonical(), &plugin.package_root) {
        ConfigurationSummary::NotDeclared => PluginConfigurationSummary::NotDeclared,
        ConfigurationSummary::Available { completeness } => PluginConfigurationSummary::Available {
            completeness: match completeness {
                ConfigurationCompleteness::Complete => PluginConfigurationCompleteness::Complete,
                ConfigurationCompleteness::Incomplete => {
                    PluginConfigurationCompleteness::Incomplete
                }
            },
        },
        ConfigurationSummary::Unavailable { error_code } => {
            PluginConfigurationSummary::Unavailable { error_code }
        }
    };

    InstalledPlugin {
        id: plugin.id.canonical(),
        namespace: plugin.id.namespace().to_string(),
        name: plugin.id.name().to_string(),
        display_name: plugin.display_name.clone(),
        version: plugin.version.to_string(),
        description: plugin.description.clone(),
        homepage: plugin.homepage.clone(),
        license: plugin.license.clone(),
        contribution,
        enabled,
        logo: plugin.logo.clone(),
        installation_validity,
        configuration,
        runtime,
    }
}
