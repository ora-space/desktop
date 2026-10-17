//! Workspace-local plugin installation. Downloading never holds a session lease lock; publishing
//! checks the lock again so a session started during the transfer cannot lose its executable.
mod catalog;
mod config;
pub use catalog::{DirectoryPluginCatalog, PluginUseLease};
pub use config::PluginConfig;
use ora_node_protocol::*;
use ora_plugin_manager::{
    ExpectedAgentDeclaration, InstallError, Installer, ResolvedReleaseSource,
};
use ora_utils::http::{DownloadError, DownloadOptions, DownloadSource, HttpDownload};
use std::path::PathBuf;

/// Executes Cloud's exact plans without consulting a marketplace or activating a plugin.
pub struct PluginInstaller<D> {
    installer: Installer<D>,
    catalog: DirectoryPluginCatalog,
    host_target: Option<ora_plugin_manager::HookTarget>,
    download_options: DownloadOptions,
}

impl<D: HttpDownload> PluginInstaller<D> {
    /// Composes one Workspace's installation owner over an injected downloader.
    pub fn new(
        home: PathBuf,
        downloader: D,
        host_target: Option<ora_plugin_manager::HookTarget>,
    ) -> Self {
        Self::with_config(home, downloader, host_target, PluginConfig::default())
    }

    /// Uses deployment-selected timing without letting Cloud's plugin plan change transfer policy.
    pub fn with_config(
        home: PathBuf,
        downloader: D,
        host_target: Option<ora_plugin_manager::HookTarget>,
        config: PluginConfig,
    ) -> Self {
        Self {
            installer: Installer::new(downloader),
            catalog: DirectoryPluginCatalog::new(home),
            host_target,
            download_options: config.download_options(),
        }
    }

    /// Shares the same use-lease registry with the Agent session host.
    pub fn catalog(&self) -> DirectoryPluginCatalog {
        self.catalog.clone()
    }

    /// Removes only this owner's temporary downloads before replaying interrupted executions.
    /// Must run before any execution or Agent session is started.
    pub fn recover(&self) -> std::io::Result<()> {
        let root = self.catalog.staging_root();
        ora_utils::path::create_directories_without_symlinks(&root)?;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("install-")
                && entry.file_type()?.is_dir()
            {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }

    /// Runs every item independently. Root failure is execution-wide; an invalid package or a
    /// failed transfer affects only that plugin, and never starts plugin code.
    pub async fn execute(
        &self,
        command: &PluginCommand,
        node: &NodeRuntimeIdentity,
    ) -> PluginExecutionResult {
        if command.validate().is_err()
            || command.node_id() != &node.node_id
            || ora_utils::path::create_directories_without_symlinks(&self.catalog.staging_root())
                .is_err()
            || ora_utils::path::create_directories_without_symlinks(
                &ora_plugin_manager::installed_root(&self.catalog.home),
            )
            .is_err()
        {
            return PluginExecutionResult::PluginsFailed(PluginsFailed {
                node: node.clone(),
                failure: PluginsFailureCode::PluginRootUnavailable,
            });
        }
        let mut items = Vec::new();
        match command {
            PluginCommand::Install(m) => {
                for plugin in &m.payload.spec.plugins {
                    let outcome = match self.install(plugin).await {
                        Ok(()) => PluginItemOutcome::Installed {
                            version: plugin.version.clone(),
                        },
                        Err(failure) => PluginItemOutcome::Failed { failure },
                    };
                    items.push(PluginItemResult {
                        plugin_id: plugin.plugin_id.clone(),
                        outcome,
                    });
                }
            }
            PluginCommand::Remove(m) => {
                for plugin in &m.payload.spec.plugins {
                    let outcome = match self.catalog.remove(&plugin.plugin_id, &plugin.version) {
                        Ok(()) => PluginItemOutcome::Removed {},
                        Err(failure) => PluginItemOutcome::Failed { failure },
                    };
                    items.push(PluginItemResult {
                        plugin_id: plugin.plugin_id.clone(),
                        outcome,
                    });
                }
            }
        }
        PluginExecutionResult::PluginsCompleted(PluginsCompleted {
            node: node.clone(),
            items,
        })
    }

    /// Verifies the planned identity before using it in a path and selects only an exact target.
    async fn install(&self, plugin: &PluginInstall) -> Result<(), PluginFailureCode> {
        let (id, version) = catalog::identity(&plugin.plugin_id, &plugin.version)?;
        if self.catalog.already_installed(&id, &version)? {
            return Ok(());
        }
        let (download, target) = match &plugin.release {
            PluginRelease::Universal { download } => (download, None),
            PluginRelease::Targets { targets } => {
                let host = self
                    .host_target
                    .as_ref()
                    .ok_or(PluginFailureCode::NoMatchingTarget)?;
                let selected = targets
                    .iter()
                    .find(|t| t.target == host.as_str())
                    .ok_or(PluginFailureCode::NoMatchingTarget)?;
                (&selected.download, Some(host.clone()))
            }
        };
        let url = url::Url::parse(&download.url).map_err(|_| PluginFailureCode::InvalidPackage)?;
        // Plans contain download locations, not credentials. In particular reqwest must not
        // turn URL user-info into an Authorization header.
        if !url.username().is_empty() || url.password().is_some() {
            return Err(PluginFailureCode::InvalidPackage);
        }
        let mut digest = [0u8; 32];
        for (i, byte) in digest.iter_mut().enumerate() {
            *byte = u8::from_str_radix(
                download
                    .sha256
                    .as_str()
                    .get(i * 2..i * 2 + 2)
                    .ok_or(PluginFailureCode::InvalidPackage)?,
                16,
            )
            .map_err(|_| PluginFailureCode::InvalidPackage)?;
        }
        let source = match target {
            Some(target) => {
                ResolvedReleaseSource::targeted(DownloadSource::Url(url), digest, target)
            }
            None => ResolvedReleaseSource::universal(DownloadSource::Url(url), digest),
        };
        // Cloud froze the declaration association into the plan when the selected catalog entry
        // declared one; an absent association installs without a declaration binding to prove.
        let declaration = plugin
            .agent_configuration
            .as_ref()
            .map(declaration_expectation)
            .transpose()?;
        let prepared = self
            .installer
            .prepare_release(
                &id,
                &version,
                source,
                &self.catalog.staging_root(),
                self.download_options,
                declaration.as_ref(),
            )
            .await
            .map_err(failure)?;
        self.catalog.publish(&id, &version, prepared)
    }
}

/// Maps the plan's frozen declaration association to the installer's expectation.
///
/// Message validation already proved the wire digest is 64 canonical lowercase hex characters,
/// which is exactly what the package world's digest type parses, so the error arm is a fail-closed
/// guard for a plan that skipped validation rather than an expected outcome.
fn declaration_expectation(
    plan: &PluginAgentConfiguration,
) -> Result<ExpectedAgentDeclaration, PluginFailureCode> {
    let digest = ora_plugin_manager::Sha256Digest::parse(plan.declaration_sha256.as_str())
        .map_err(|_| PluginFailureCode::InvalidPackage)?;
    Ok(ExpectedAgentDeclaration::new(plan.schema_version, digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canonical wire digest: 64 lowercase hexadecimal characters.
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A plan association maps losslessly onto the installer's expectation.
    #[test]
    fn maps_a_plan_association_to_the_installer_expectation() {
        let expectation = declaration_expectation(&PluginAgentConfiguration {
            schema_version: 2,
            declaration_sha256: Sha256Digest::new(DIGEST),
        })
        .unwrap();
        assert_eq!(expectation.schema_version(), 2);
        assert_eq!(expectation.declaration_sha256().to_string(), DIGEST);
    }

    /// A digest that is not canonical hexadecimal fails the plan as an invalid package instead of
    /// installing with a binding nobody could prove.
    #[test]
    fn rejects_a_non_canonical_digest() {
        assert_eq!(
            declaration_expectation(&PluginAgentConfiguration {
                schema_version: 2,
                declaration_sha256: Sha256Digest::new("not a digest"),
            }),
            Err(PluginFailureCode::InvalidPackage)
        );
    }
}

/// Keeps raw network/path diagnostics out of Cloud's bounded failure vocabulary.
fn failure(error: InstallError) -> PluginFailureCode {
    match error {
        InstallError::ChecksumMismatch { .. } => PluginFailureCode::ChecksumMismatch,
        InstallError::Download(error) => match *error {
            DownloadError::ChecksumMismatch { .. } => PluginFailureCode::ChecksumMismatch,
            DownloadError::Io { .. } => PluginFailureCode::InstallFailed,
            DownloadError::Network { .. }
            | DownloadError::HttpStatus { .. }
            | DownloadError::TooLarge { .. }
            | DownloadError::Timeout { .. }
            | DownloadError::Cancelled
            | DownloadError::InvalidSource(_) => PluginFailureCode::DownloadFailed,
        },
        InstallError::Io { .. } | InstallError::AlreadyInstalled { .. } => {
            PluginFailureCode::InstallFailed
        }
        InstallError::NoArtifactForTarget { .. } | InstallError::UnsupportedHost => {
            PluginFailureCode::NoMatchingTarget
        }
        InstallError::Extract { .. }
        | InstallError::MissingManifest
        | InstallError::InvalidManifest(_)
        | InstallError::InvalidPackage { .. }
        | InstallError::MissingRelease
        | InstallError::TargetMismatch { .. }
        | InstallError::MissingArtifactTarget => PluginFailureCode::InvalidPackage,
    }
}
