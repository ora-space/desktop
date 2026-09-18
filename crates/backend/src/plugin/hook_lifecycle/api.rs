//! What the plugin API exposes to the operations layer for Hook lifecycle commands.
//!
//! The executor in the parent module runs one declared command against one installed package. This
//! module decides *which* package that is and *which* operations may trigger it, so the
//! authorization boundary (D4) sits with the resolution it authorizes instead of in the plugin
//! API's own file, which is about discovery and process lifecycle.

#[cfg(test)]
use super::HookCommandRunner;
use super::InstalledHook;
use crate::error::{BackendError, ErrorClassification};
use crate::plugin::PluginApi;
use ora_contracts::{
    EmptyErrorParams, HookLifecyclePhase, HookLifecycleReport, InitializeHookRequest,
    InitializeHookResponse, PublicError,
};
use ora_domain::PluginId;
use ora_logging::ora_warn;
use ora_plugin_manager::PluginContribution;

impl PluginApi {
    /// Runs the `init` lifecycle command of one freshly installed, imported, or updated package.
    ///
    /// Called by the single-plugin operations only. A pack member lands through the same
    /// finalization but never through this method: the pack installs packages, and running a
    /// program one of them ships needs the user's separate authorization for that member (D2).
    ///
    /// The result is logged and remembered rather than returned: `init` failing keeps the plugin
    /// installed and is surfaced through the lifecycle report, so it must not turn a successful
    /// install into an error the caller would report as a failed install (D8).
    pub(crate) async fn initialize_installed_hook(&self, plugin_id: &str) {
        let Some(hook) = self.installed_hook(plugin_id) else {
            return;
        };
        self.hook_lifecycle
            .execute(&hook, HookLifecyclePhase::Init)
            .await;
    }

    /// Runs the `deinit` lifecycle command of one package that is about to be removed.
    ///
    /// Called before the package directory is staged away, because a lifecycle command runs the
    /// package's own executable: after the removal begins, the file the tool needs is already
    /// gone. A package that declares no `deinit` runs nothing and the uninstall continues, which
    /// is what the declaration's optionality means.
    pub(crate) async fn deinitialize_installed_hook(&self, plugin_id: &str) {
        let Some(hook) = self.installed_hook(plugin_id) else {
            return;
        };
        self.hook_lifecycle
            .execute(&hook, HookLifecyclePhase::Deinit)
            .await;
    }

    /// Returns this session's Hook lifecycle results for the settings surface to merge by plugin.
    pub(crate) fn hook_lifecycle_reports(&self) -> Vec<HookLifecycleReport> {
        self.hook_lifecycle.reports()
    }

    /// Runs one user-requested `init` and reports its outcome.
    ///
    /// Unlike the automatic trigger, this is an explicit request for one named Hook, so every
    /// reason nothing can run is returned as an error: the user picked this plugin and pressed
    /// the action, and a silent no-op would look like a successful initialization.
    pub(crate) async fn initialize_hook(
        &self,
        request: InitializeHookRequest,
    ) -> Result<InitializeHookResponse, BackendError> {
        if !request.hook_execution_acknowledged {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "initializing a Hook runs a program from the installed package, which the request did not authorize",
            ));
        }
        let Some(hook) = self.installed_hook(&request.plugin_id) else {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "the plugin is not an installed Hook package",
            ));
        };
        let report = self
            .hook_lifecycle
            .execute(&hook, HookLifecyclePhase::Init)
            .await
            .ok_or_else(|| {
                BackendError::internal(
                    "an installed Hook must declare an init command",
                    std::io::Error::new(std::io::ErrorKind::InvalidData, request.plugin_id.clone()),
                )
            })?;
        Ok(InitializeHookResponse { report })
    }

    /// Resolves one installed package into the Hook it declares, if it declares one.
    ///
    /// Returns `None` for a package that is installed but is not a Hook, which is every other
    /// kind and not an error. A package the snapshot does not contain is logged and also reported
    /// as `None`: the automatic triggers run after a successful operation that already refreshed
    /// the snapshot, so a miss there means the snapshot could not be refreshed, and failing the
    /// user's install over it would be the wrong trade.
    fn installed_hook(&self, plugin_id: &str) -> Option<InstalledHook> {
        let Ok(id) = PluginId::parse(plugin_id) else {
            ora_warn!(
                plugin_id,
                "skipping Hook lifecycle: the plugin id is invalid"
            );
            return None;
        };
        let Some(plugin) = self.lifecycle.installed_plugin(&id) else {
            ora_warn!(
                plugin_id,
                "skipping Hook lifecycle: the installed snapshot has no such plugin"
            );
            return None;
        };
        let PluginContribution::Hook(descriptor) = &plugin.contributes else {
            return None;
        };
        Some(InstalledHook {
            plugin_id: plugin.id.canonical(),
            package_root: plugin.package_root.clone(),
            descriptor: descriptor.clone(),
        })
    }

    /// Substitutes the Hook lifecycle command runner so a test never spawns a package program.
    #[cfg(test)]
    pub(crate) fn install_hook_command_runner(&self, runner: impl HookCommandRunner) {
        self.hook_lifecycle.install_runner(runner);
    }
}
