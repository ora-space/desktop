use crate::PluginLifecycleError;
use ora_application::PluginStateRepository;
use ora_contracts::{InstalledPlugin, InstalledPluginAgent, PluginRuntimeStatus};
use ora_domain::{PluginEnabledState, PluginId};
use ora_plugin_manager::InstalledPlugin as DiscoveredPlugin;
use std::collections::{BTreeMap, BTreeSet};

/// Holds the filesystem snapshot and process-scoped lifecycle state as one atomic view.
pub(super) struct LifecycleState<Runtime> {
    pub(super) installed: Vec<DiscoveredPlugin>,
    pub(super) managed_by_id: BTreeMap<PluginId, ManagedPluginState<Runtime>>,
    pub(super) next_attempt: u64,
}

/// Makes the illegal combination of a disabled plugin with a live runtime unrepresentable.
pub(super) enum ManagedPluginState<Runtime> {
    Disabled,
    Enabled(EnabledRuntime<Runtime>),
}

/// Represents every process-scoped state available only to an enabled plugin.
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
        .map(|plugin| PluginId::new(&plugin.id))
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

    let ora_plugin_manager::PluginContribution::Agent(agent) = &plugin.contributes;

    InstalledPlugin {
        id: plugin.id.clone(),
        package_name: plugin.package_name.clone(),
        display_name: plugin.display_name.clone(),
        version: plugin.version.to_string(),
        kind: plugin.contributes.kind().to_string(),
        main: plugin.main.as_str().to_string(),
        agent: InstalledPluginAgent {
            display_name: agent.display_name.clone(),
            contract_version: agent.contract_version,
        },
        enabled,
        logo: plugin.logo.clone(),
        runtime,
    }
}
