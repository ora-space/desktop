//! Marketplace package transfer: release resolution, download, install, and update.
//!
//! Install and update share one release-resolution and installer-construction path so a source's
//! namespace, proxy policy, and object-store scoping can never diverge between the two operations.
//! Both expose an observed variant that forwards byte-level transfer progress to the host shell.

use super::PluginApi;
use crate::error::{BackendError, ErrorClassification};
use crate::proxy;
use ora_contracts::{
    EmptyErrorParams, InstallOutcome, InstallPluginRequest, InstallPluginResponse, PublicError,
    StopPluginRequest, UpdatePluginRequest, UpdatePluginResponse,
};
use ora_domain::{PluginId, PluginNamespace};
use ora_logging::ora_info;
use ora_plugin_manager::{HostTarget, InstallError, Installer, UpdateError, select_release};
use ora_plugin_manifest::{PluginKind, PluginManifest};
use ora_plugin_registry::RegistryIndex;
#[cfg(test)]
use ora_utils::http::{DownloadSource, LocalFileDownloader};
use ora_utils::http::{
    HttpDownload, ProgressCallback, ProxyConfig, ReqwestDownloader, S3AwareDownloader, S3Config,
};

impl PluginApi {
    /// Installs a marketplace plugin by resolving its release manifest from the synced sources and
    /// downloading, verifying, and extracting its package through the network-backed installer.
    ///
    /// The source registries are read only for the release `url`/`sha256` (the cached index
    /// carries display fields only), so this returns NotFound when the identifier is not in any
    /// checkout.
    pub(crate) async fn install(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, BackendError> {
        self.install_package(request, /*progress*/ None).await
    }

    /// Installs a marketplace plugin and forwards network transfer progress to the host shell.
    pub(crate) async fn install_with_progress(
        &self,
        request: InstallPluginRequest,
        progress: ProgressCallback,
    ) -> Result<InstallPluginResponse, BackendError> {
        self.install_package(request, Some(progress)).await
    }

    /// Keeps release resolution and finalization identical for observed and unobserved installs.
    async fn install_package(
        &self,
        request: InstallPluginRequest,
        progress: Option<ProgressCallback>,
    ) -> Result<InstallPluginResponse, BackendError> {
        let (manifest, namespace, use_proxy, s3_config) =
            self.resolve_marketplace_release(&request.plugin_id).await?;
        // A pack is an orchestration entry: it resolves like any listing but expands into member
        // installs instead of a release download (extension-pack decision D5).
        if matches!(manifest.kind(), PluginKind::Pack) {
            #[cfg(test)]
            if self.has_local_pack_release(&manifest, &namespace)? {
                return self
                    .install_pack_from_local_releases(request, manifest, namespace, progress)
                    .await;
            }
            return self
                .install_pack(request, manifest, namespace, use_proxy, s3_config, progress)
                .await;
        }
        let release_source = self.select_marketplace_release(&manifest)?;
        #[cfg(test)]
        if let Some(artifact) =
            self.local_marketplace_release_for_manifest(&manifest, &namespace)?
        {
            let release_source = local_release_source(release_source, artifact);
            return self
                .install_resolved_package(
                    request,
                    manifest,
                    namespace,
                    release_source,
                    &Installer::new(LocalFileDownloader),
                    progress,
                )
                .await;
        }
        let installer = self.marketplace_installer(use_proxy, s3_config).await?;
        self.install_resolved_package(
            request,
            manifest,
            namespace,
            release_source,
            &installer,
            progress,
        )
        .await
    }

    /// Installs one already-resolved ordinary package through the shared production finalization.
    async fn install_resolved_package<D>(
        &self,
        request: InstallPluginRequest,
        manifest: PluginManifest,
        namespace: PluginNamespace,
        release_source: ora_plugin_manager::ResolvedReleaseSource,
        installer: &Installer<D>,
        progress: Option<ProgressCallback>,
    ) -> Result<InstallPluginResponse, BackendError>
    where
        D: HttpDownload,
    {
        match release_source.download() {
            ora_utils::http::DownloadSource::Url(url) => {
                ora_info!(plugin_id = %request.plugin_id, url = %url, "installing marketplace plugin");
            }
            ora_utils::http::DownloadSource::Local(path) => {
                ora_info!(plugin_id = %request.plugin_id, path = %path.display(), "installing marketplace plugin from local source");
            }
            ora_utils::http::DownloadSource::S3 { key } => {
                ora_info!(plugin_id = %request.plugin_id, key = %key, "installing marketplace plugin from object store");
            }
        }
        match progress {
            Some(progress) => {
                installer
                    .install_with_progress(
                        &manifest,
                        &namespace,
                        release_source,
                        &self.home_directory,
                        progress,
                    )
                    .await
            }
            None => {
                installer
                    .install(&manifest, &namespace, release_source, &self.home_directory)
                    .await
            }
        }
        .map_err(|error| self.map_install_error("failed to install plugin", error))?;
        self.finalize_new_install(&request.plugin_id).await?;
        ora_info!(plugin_id = %request.plugin_id, "installed marketplace plugin");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
            outcome: InstallOutcome::Installed,
        })
    }

    /// Updates one installed marketplace plugin to the version its source publishes.
    ///
    /// The source resolution and proxy policy are identical to an install: the winning
    /// marketplace source decides whether the download goes through the configured proxy. The
    /// running process is stopped before its package is replaced, and the installed snapshot is
    /// rescanned afterwards so the new version becomes effective without a restart.
    pub(crate) async fn update(
        &self,
        request: UpdatePluginRequest,
    ) -> Result<UpdatePluginResponse, BackendError> {
        self.update_package(request, /*progress*/ None).await
    }

    /// Updates a marketplace plugin and forwards network transfer progress to the host shell.
    pub(crate) async fn update_with_progress(
        &self,
        request: UpdatePluginRequest,
        progress: ProgressCallback,
    ) -> Result<UpdatePluginResponse, BackendError> {
        self.update_package(request, Some(progress)).await
    }

    /// Keeps release resolution, process stopping, and finalization identical for observed and
    /// unobserved updates.
    async fn update_package(
        &self,
        request: UpdatePluginRequest,
        progress: Option<ProgressCallback>,
    ) -> Result<UpdatePluginResponse, BackendError> {
        let (manifest, namespace, use_proxy, s3_config) =
            self.resolve_marketplace_release(&request.plugin_id).await?;
        // A pack is never installed, so it has nothing to update; refreshing members means
        // installing the pack again (extension-pack decision D7).
        if matches!(manifest.kind(), PluginKind::Pack) {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "a pack cannot be updated; install the pack again to refresh its members",
            ));
        }
        let release_source = self.select_marketplace_release(&manifest)?;
        #[cfg(test)]
        if let Some(artifact) =
            self.local_marketplace_release_for_manifest(&manifest, &namespace)?
        {
            let release_source = local_release_source(release_source, artifact);
            return self
                .update_resolved_package(
                    request,
                    manifest,
                    namespace,
                    release_source,
                    &Installer::new(LocalFileDownloader),
                    progress,
                )
                .await;
        }
        let installer = self.marketplace_installer(use_proxy, s3_config).await?;
        self.update_resolved_package(
            request,
            manifest,
            namespace,
            release_source,
            &installer,
            progress,
        )
        .await
    }

    /// Updates one already-resolved ordinary package through the shared lifecycle finalization.
    async fn update_resolved_package<D>(
        &self,
        request: UpdatePluginRequest,
        manifest: PluginManifest,
        namespace: PluginNamespace,
        release_source: ora_plugin_manager::ResolvedReleaseSource,
        installer: &Installer<D>,
        progress: Option<ProgressCallback>,
    ) -> Result<UpdatePluginResponse, BackendError>
    where
        D: HttpDownload,
    {
        match release_source.download() {
            ora_utils::http::DownloadSource::Url(url) => {
                ora_info!(plugin_id = %request.plugin_id, url = %url, "updating marketplace plugin");
            }
            ora_utils::http::DownloadSource::Local(path) => {
                ora_info!(plugin_id = %request.plugin_id, path = %path.display(), "updating marketplace plugin from local source");
            }
            ora_utils::http::DownloadSource::S3 { key } => {
                ora_info!(plugin_id = %request.plugin_id, key = %key, "updating marketplace plugin from object store");
            }
        }
        // The package directory is replaced while the plugin may be running, so the process is
        // stopped first; stopping a webview/skill/MCP/hook package is a no-op.
        self.lifecycle
            .stop_plugin(StopPluginRequest {
                plugin_id: request.plugin_id.clone(),
            })
            .await
            .map_err(BackendError::from)?;
        match progress {
            Some(progress) => {
                installer
                    .update_with_progress(
                        &manifest,
                        &namespace,
                        release_source,
                        &self.home_directory,
                        progress,
                    )
                    .await
            }
            None => {
                installer
                    .update(&manifest, &namespace, release_source, &self.home_directory)
                    .await
            }
        }
        .map_err(|error| self.map_update_error("failed to update plugin", error))?;
        self.finalize_new_install(&request.plugin_id).await?;
        ora_info!(plugin_id = %request.plugin_id, "updated marketplace plugin");
        Ok(UpdatePluginResponse {
            plugin_id: request.plugin_id,
        })
    }

    /// Resolves the release manifest for one marketplace identifier across the configured sources.
    ///
    /// The id names the namespace of the source that published it, so only that source can
    /// answer: the returned namespace and proxy policy always belong to the entry's own
    /// repository, and an install or update can never be redirected by reordering the source list
    /// or by another source publishing the same `identifier`.
    async fn resolve_marketplace_release(
        &self,
        plugin_id: &str,
    ) -> Result<(PluginManifest, PluginNamespace, bool, Option<S3Config>), BackendError> {
        let proxy_settings = self.settings.network_proxy_settings().await?;
        let registry_sources = self.prepared_registry_sources(proxy_settings)?;
        // A malformed identifier can never name a registry entry, so it is reported the same way
        // as an unknown one instead of leaking the id grammar as a separate error class.
        let plugin_id = PluginId::parse(plugin_id).map_err(|_| {
            BackendError::new(
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
                "marketplace plugin id is not a valid `<namespace>/<name>`",
            )
        })?;
        for (source, use_proxy, s3_config) in &registry_sources {
            if let Some(manifest) =
                RegistryIndex::resolve_manifest(source, &plugin_id).map_err(|error| {
                    BackendError::internal("failed to resolve plugin release manifest", error)
                })?
            {
                return Ok((
                    manifest,
                    source.namespace().clone(),
                    *use_proxy,
                    s3_config.clone(),
                ));
            }
        }
        Err(BackendError::new(
            ErrorClassification::NotFound,
            PublicError::PluginNotFound(EmptyErrorParams {}),
            "marketplace plugin was not found in the registry",
        ))
    }

    /// Selects the downloadable release for `manifest` against the current host.
    ///
    /// Universal releases ignore the host. Targeted Hook releases require a supported triple and
    /// an exact artifact match so a wrong-architecture package is refused before download.
    fn select_marketplace_release(
        &self,
        manifest: &PluginManifest,
    ) -> Result<ora_plugin_manager::ResolvedReleaseSource, BackendError> {
        let host_target = ora_plugin_registry::current_host_target();
        select_release(manifest, HostTarget::from_option(host_target.as_ref()))
            .map_err(|error| self.map_install_error("failed to select plugin release", error))
    }

    /// Returns whether a pack has any test-local member transfer overrides.
    #[cfg(test)]
    fn has_local_pack_release(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
    ) -> Result<bool, BackendError> {
        let Some(pack) = manifest.pack() else {
            return Ok(false);
        };
        for member in pack.members() {
            let member_id = PluginId::new(namespace.clone(), member.identifier().as_str())
                .map_err(|error| BackendError::internal("invalid pack member id", error))?;
            if self.local_marketplace_release(&member_id).is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Resolves the test-local transfer override for an ordinary marketplace manifest.
    #[cfg(test)]
    fn local_marketplace_release_for_manifest(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
    ) -> Result<Option<std::path::PathBuf>, BackendError> {
        let plugin_id = PluginId::new(namespace.clone(), manifest.name().as_str())
            .map_err(|error| BackendError::internal("invalid marketplace plugin id", error))?;
        Ok(self.local_marketplace_release(&plugin_id))
    }

    /// Maps installer failures that describe host incompatibility onto the public contract error.
    pub(super) fn map_install_error(
        &self,
        context: &'static str,
        error: InstallError,
    ) -> BackendError {
        match error {
            InstallError::NoArtifactForTarget { .. }
            | InstallError::MissingRelease
            | InstallError::UnsupportedHost
            | InstallError::TargetMismatch { .. }
            | InstallError::MissingArtifactTarget => BackendError::new(
                ErrorClassification::Unprocessable,
                PublicError::PluginHostIncompatible(EmptyErrorParams {}),
                format!("{error}"),
            ),
            error => BackendError::internal(context, error),
        }
    }

    /// Maps update failures, preserving host-incompatibility from the nested install path.
    fn map_update_error(&self, context: &'static str, error: UpdateError) -> BackendError {
        match error {
            UpdateError::Install(install_error) => self.map_install_error(context, install_error),
            error => BackendError::internal(context, error),
        }
    }

    /// Returns the downloader proxy configuration for one marketplace source's proxy policy.
    async fn download_proxy_for(&self, use_proxy: bool) -> Result<ProxyConfig, BackendError> {
        if !use_proxy {
            return Ok(ProxyConfig::default());
        }
        let proxy_settings = self.settings.network_proxy_settings().await?;
        proxy::download_proxy(proxy_settings.as_ref())?.ok_or_else(|| {
            BackendError::invalid_proxy_settings(
                "a marketplace source uses the proxy but no proxy is configured",
            )
        })
    }

    /// Builds a source-scoped downloader that signs only the configured S3 endpoint and bucket.
    pub(super) async fn marketplace_installer(
        &self,
        use_proxy: bool,
        s3_config: Option<S3Config>,
    ) -> Result<Installer<S3AwareDownloader>, BackendError> {
        let download_proxy = self.download_proxy_for(use_proxy).await?;
        Ok(Installer::new(S3AwareDownloader::new(
            ReqwestDownloader::new(download_proxy),
            s3_config,
        )))
    }
}

/// Replaces only the selected transfer locator while preserving digest and target verification.
#[cfg(test)]
fn local_release_source(
    source: ora_plugin_manager::ResolvedReleaseSource,
    artifact: std::path::PathBuf,
) -> ora_plugin_manager::ResolvedReleaseSource {
    let digest = *source.sha256();
    match source.target().cloned() {
        Some(target) => ora_plugin_manager::ResolvedReleaseSource::targeted(
            DownloadSource::Local(artifact),
            digest,
            target,
        ),
        None => ora_plugin_manager::ResolvedReleaseSource::universal(
            DownloadSource::Local(artifact),
            digest,
        ),
    }
}
