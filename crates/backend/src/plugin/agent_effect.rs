//! Single Agent Effect declaration snapshot owned by `PluginApi`.
//!
//! Skill surfaces and MCP capability ride on the same per-plugin map so convergence has one source
//! to read. The Skill worker still projects filesystem surfaces from that snapshot; MCP
//! materialization stays on the later Agent Target worker.

use super::PluginApi;
use crate::agent_runtime::plugin_agent::AgentPluginEffectDeclaration;
use crate::error::BackendError;
use ora_application::Clock;
use ora_contracts::ListInstalledPluginsRequest;
use ora_domain::{PluginId, WorkspaceLocation};
use ora_effect::SurfaceDescriptorSet;
use std::path::Path;
use std::sync::PoisonError;

impl PluginApi {
    /// Returns the exact installed package version used to bind Agent Capability Revision.
    pub(crate) fn installed_plugin_version(&self, plugin_id: &PluginId) -> Option<String> {
        self.list(ListInstalledPluginsRequest {})
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == plugin_id.canonical())
            .map(|plugin| plugin.version)
    }

    /// Replaces one Agent plugin's Skill surfaces, MCP capability, and capability revision.
    ///
    /// This is the single declaration snapshot the Skill worker and later MCP worker both read.
    /// An absent declaration removes the plugin so uninstall cannot leave a ghost consumer.
    pub(crate) fn replace_agent_plugin_declaration(
        &self,
        plugin_id: PluginId,
        declaration: AgentPluginEffectDeclaration,
    ) -> Result<(), BackendError> {
        let descriptors = {
            let mut registered = self
                .agent_effect_surfaces
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if declaration.is_absent() {
                registered.remove(&plugin_id);
            } else {
                registered.insert(plugin_id, declaration);
            }
            registered
                .values()
                .flat_map(|declaration| declaration.skill_surfaces.iter().cloned())
                .collect::<Vec<_>>()
        };
        let timestamp = self.clock.now_timestamp_millis();
        let workspaces = self
            .workspace_repository
            .list_all_workspaces()
            .map_err(|error| BackendError::internal("failed to list Effect Workspaces", error))?;
        for workspace in workspaces {
            let WorkspaceLocation::LocalFilesystem { path } = &workspace.location else {
                // The first adapter is deliberately filesystem-only. Remote Workspaces need a
                // provider-owned adapter instead of treating an opaque locator as a host path.
                continue;
            };
            let merged = SurfaceDescriptorSet::merge(&workspace.id, descriptors.clone())
                .map_err(|error| BackendError::internal("invalid Agent Effect surface", error))?;
            self.effect_repository
                .replace_surfaces(&workspace.id, Path::new(path), &merged, timestamp)
                .map_err(|error| {
                    BackendError::internal("failed to persist Agent Effect surfaces", error)
                })?;
        }
        // Waking after the commit, never before it: the request the worker will read is already
        // durable, so a wake lost to a crash costs a scan interval rather than a reconcile.
        if let Some(reconcile) = self.effect_reconcile.get() {
            reconcile.notify();
        }
        Ok(())
    }

    /// Returns the merged Agent Effect declarations of every currently registered Agent plugin.
    ///
    /// This snapshot is the single source Skill-surface convergence reads. Callers that only
    /// persist filesystem Skill surfaces flatten `skill_surfaces`; MCP capability and revision stay
    /// on the same entries so a later worker cannot miss a second registry. The map is
    /// process-local: a plugin that is not running declares nothing.
    pub(crate) fn agent_effect_surface_declarations(
        &self,
    ) -> Vec<(PluginId, AgentPluginEffectDeclaration)> {
        self.agent_effect_surfaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(plugin_id, declaration)| (plugin_id.clone(), declaration.clone()))
            .collect()
    }
}
