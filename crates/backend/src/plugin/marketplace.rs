//! Marketplace package transfer: release resolution, download, install, and update.
//!
//! Install and update share one release-resolution and installer-construction path so a source's
//! namespace, proxy policy, and object-store scoping can never diverge between the two operations.

use super::PluginApi;
use crate::error::{BackendError, ErrorClassification};
use crate::proxy;
use ora_contracts::{
    EmptyErrorParams, InstallPluginRequest, InstallPluginResponse, PublicError, StopPluginRequest,
    UpdatePluginRequest, UpdatePluginResponse,
};
use ora_domain::{PluginId, PluginNamespace};
use ora_logging::ora_info;
use ora_plugin_manager::{HostTarget, InstallError, Installer, UpdateError, select_release};
use ora_plugin_manifest::PluginManifest;
use ora_plugin_registry::RegistryIndex;
use ora_utils::http::{
    ProgressCallback, ProxyConfig, ReqwestDownloader, S3AwareDownloader, S3Config,
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
            self.resolve_marketplace_release(&request.plugin_id)?;
        let release_source = self.select_marketplace_release(&manifest)?;
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
        let installer = self.marketplace_installer(use_proxy, s3_config)?;
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
        let outcome = self.finalize_new_install(&request.plugin_id).await?;
        ora_info!(plugin_id = %request.plugin_id, outcome = ?outcome, "installed marketplace plugin");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
            outcome,
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
        let (manifest, namespace, use_proxy, s3_config) =
            self.resolve_marketplace_release(&request.plugin_id)?;
        let release_source = self.select_marketplace_release(&manifest)?;
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
        self.marketplace_installer(use_proxy, s3_config)?
            .update(&manifest, &namespace, release_source, &self.home_directory)
            .await
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
    fn resolve_marketplace_release(
        &self,
        plugin_id: &str,
    ) -> Result<(PluginManifest, PluginNamespace, bool, Option<S3Config>), BackendError> {
        let registry_sources = self.prepared_registry_sources()?;
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

    /// Maps installer failures that describe host incompatibility onto the public contract error.
    fn map_install_error(&self, context: &'static str, error: InstallError) -> BackendError {
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
    fn download_proxy_for(&self, use_proxy: bool) -> Result<ProxyConfig, BackendError> {
        if !use_proxy {
            return Ok(ProxyConfig::default());
        }
        let proxy_settings = self.settings.network_proxy_settings()?;
        proxy::download_proxy(proxy_settings.as_ref())?.ok_or_else(|| {
            BackendError::invalid_proxy_settings(
                "a marketplace source uses the proxy but no proxy is configured",
            )
        })
    }

    /// Builds a source-scoped downloader that signs only the configured S3 endpoint and bucket.
    fn marketplace_installer(
        &self,
        use_proxy: bool,
        s3_config: Option<S3Config>,
    ) -> Result<Installer<S3AwareDownloader>, BackendError> {
        let download_proxy = self.download_proxy_for(use_proxy)?;
        Ok(Installer::new(S3AwareDownloader::new(
            ReqwestDownloader::new(download_proxy),
            s3_config,
        )))
    }
}
