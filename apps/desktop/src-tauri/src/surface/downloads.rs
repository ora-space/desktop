//! Download delivery: reservation into the plugin's `downloads/` directory, frontend events,
//! and the `ui/downloadCompleted` call into the plugin process.

use crate::surface::effects::emit_event;
use crate::surface::gateway::{SurfaceConnection, SurfacePluginGateway};
use crate::surface::hooks::DownloadSink;
use crate::surface::plugin_link::PROCESS_START_WAIT;
use ora_logging::{ora_info, ora_warn};
use ora_plugin_lifecycle::UI_DOWNLOAD_COMPLETED_METHOD;
use ora_surface::{
    CompletedDownload, DownloadAcceptance, DownloadClock, DownloadCoordinator, DownloadFinish,
    DownloadStatus, SurfaceEvent, SurfaceRecord, SurfaceRegistry, SurfaceState, WebviewLabel,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager, Runtime, Url};
use time::format_description::well_known::Rfc3339;
use tokio::sync::Semaphore;

/// Host-written child of the plugin data directory; mirrors `PluginDataDirectories::ensure`.
const DOWNLOADS_DIRECTORY: &str = "downloads";

/// Upper bound on concurrent `ui/downloadCompleted` deliveries; excess deliveries queue.
const MAX_CONCURRENT_DISPATCHES: usize = 8;

/// Routes download events of every surface webview.
///
/// The destination is decided solely by the webview label resolved through the registry, so a
/// remote page can never steer a file into another plugin's directory.
pub struct DownloadDispatcher<G, R: Runtime, C: DownloadClock> {
    registry: Arc<SurfaceRegistry>,
    coordinator: DownloadCoordinator<C>,
    gateway: G,
    app: AppHandle<R>,
    permits: Arc<Semaphore>,
}

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock> DownloadDispatcher<G, R, C> {
    /// The clock stamps reservations and completions; production passes the process-local
    /// clock, tests a fixed instant.
    pub fn new(registry: Arc<SurfaceRegistry>, gateway: G, app: AppHandle<R>, clock: C) -> Self {
        Self {
            registry,
            coordinator: DownloadCoordinator::new(clock),
            gateway,
            app,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_DISPATCHES)),
        }
    }

    /// Reserves a `.part` path and redirects the browser engine to it.
    fn requested(
        &self,
        record: &SurfaceRecord,
        page_url: Option<Url>,
        url: &Url,
        destination: &mut std::path::PathBuf,
    ) -> bool {
        let plugin_id = &record.definition.id.plugin_id;
        let directory = match self.gateway.data_directory(plugin_id) {
            Ok(directory) => directory.join(DOWNLOADS_DIRECTORY),
            Err(error) => {
                ora_warn!(message = "plugin data directory unavailable for download", plugin_id = %plugin_id, error = %error);
                return false;
            }
        };
        match self
            .coordinator
            .request(&record.label, &directory, url, page_url, destination)
        {
            Ok(DownloadAcceptance::Accepted {
                id,
                file_name,
                part_path,
            }) => {
                ora_info!(message = "surface download started", plugin_id = %plugin_id, download_id = id.value(), url = %url, part_path = %part_path.display());
                *destination = part_path;
                emit_event(
                    &self.app,
                    &SurfaceEvent::DownloadStarted {
                        instance: record.instance.value(),
                        plugin_id: plugin_id.to_string(),
                        file_name,
                    },
                );
                true
            }
            Ok(DownloadAcceptance::Rejected(reason)) => {
                ora_warn!(message = "surface download rejected", plugin_id = %plugin_id, url = %url, reason = ?reason);
                false
            }
            Err(error) => {
                ora_warn!(message = "surface download reservation failed", plugin_id = %plugin_id, url = %url, error = %error);
                false
            }
        }
    }

    /// Promotes or discards the `.part` file, notifies the frontend, and dispatches to the plugin.
    fn finished(&self, record: &SurfaceRecord, url: &Url, status: DownloadStatus) -> bool {
        let plugin_id = record.definition.id.plugin_id.to_string();
        let instance = record.instance.value();
        match self.coordinator.finish(&record.label, url, status) {
            Ok(DownloadFinish::Completed(download)) => {
                emit_event(
                    &self.app,
                    &SurfaceEvent::DownloadCompleted {
                        instance,
                        plugin_id,
                        file_name: download.file_name.clone(),
                        path: download.path.display().to_string(),
                    },
                );
                // An embedded surface lives inside the main window, which is therefore already
                // in front; only a separate window can hide the completion toast.
                if matches!(record.state, SurfaceState::Windowed { .. }) {
                    bring_main_window_forward(&self.app);
                }
                self.dispatch(record.clone(), *download);
            }
            Ok(DownloadFinish::Failed { id, file_name }) => {
                ora_warn!(message = "surface download failed", plugin_id, download_id = id.value(), url = %url);
                emit_event(
                    &self.app,
                    &SurfaceEvent::DownloadFailed {
                        instance,
                        plugin_id,
                        file_name,
                        reason: "the browser engine reported a failed transfer".to_owned(),
                    },
                );
            }
            Ok(DownloadFinish::Unknown) => {
                ora_warn!(message = "surface download finished without a reservation", plugin_id, url = %url);
            }
            Err(error) => {
                ora_warn!(message = "surface download could not be finalized", plugin_id = %plugin_id, url = %url, error = %error);
                emit_event(
                    &self.app,
                    &SurfaceEvent::DownloadFailed {
                        instance,
                        plugin_id,
                        file_name: url
                            .path_segments()
                            .and_then(Iterator::last)
                            .unwrap_or("")
                            .to_owned(),
                        reason: "the downloaded file could not be moved into place".to_owned(),
                    },
                );
            }
        }
        true
    }

    /// Hands the completed file to the plugin process off the hook thread.
    ///
    /// The plugin is started if needed; a plugin that cannot start keeps the file on disk and
    /// only loses the notification, which is logged as unhandled.
    fn dispatch(&self, record: SurfaceRecord, download: CompletedDownload) {
        let gateway = self.gateway.clone();
        let permits = self.permits.clone();
        tauri::async_runtime::spawn(async move {
            // The semaphore is never closed, so acquisition cannot fail.
            let _permit = permits.acquire().await;
            let plugin_id = record.definition.id.plugin_id.clone();
            let connection = match gateway.ensure_running(&plugin_id, PROCESS_START_WAIT).await {
                Ok(connection) => connection,
                Err(error) => {
                    ora_warn!(message = "download unhandled: plugin process unavailable", plugin_id = %plugin_id, download_id = download.id.value(), path = %download.path.display(), reason = %error);
                    return;
                }
            };
            let params = download_completed_params(&record, connection.generation().0, &download);
            match connection
                .invoke(UI_DOWNLOAD_COMPLETED_METHOD, params)
                .await
            {
                Ok(_) => {
                    ora_info!(message = "download delivered to plugin", plugin_id = %plugin_id, download_id = download.id.value())
                }
                Err(error) => {
                    ora_warn!(message = "plugin rejected download", plugin_id = %plugin_id, surface_id = record.definition.id.surface_id.as_str(), download_id = download.id.value(), error = %error)
                }
            }
        });
    }
}

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock + Send + Sync + 'static> DownloadSink<R>
    for DownloadDispatcher<G, R, C>
{
    fn handle(
        &self,
        label: &WebviewLabel,
        page_url: Option<Url>,
        event: DownloadEvent<'_>,
    ) -> bool {
        let Some(record) = self.registry.resolve_label(label.as_str()) else {
            ora_warn!(message = "download event from an unregistered webview", label = %label);
            return false;
        };
        match event {
            DownloadEvent::Requested { url, destination } => {
                self.requested(&record, page_url, &url, destination)
            }
            DownloadEvent::Finished { url, success, .. } => {
                let status = if success {
                    DownloadStatus::Succeeded
                } else {
                    DownloadStatus::Failed
                };
                self.finished(&record, &url, status)
            }
            // `DownloadEvent` is `#[non_exhaustive]`; unknown future events must not block.
            _ => true,
        }
    }
}

/// Builds the `ui/downloadCompleted` params: session identity plus the durable file facts.
pub fn download_completed_params(
    record: &SurfaceRecord,
    generation: u64,
    download: &CompletedDownload,
) -> Value {
    json!({
        "surfaceId": record.definition.id.surface_id.as_str(),
        "instanceId": record.instance.value(),
        "generation": generation,
        "download": {
            "id": download.id.value(),
            "pageUrl": download.page_url.as_ref().map(Url::as_str),
            "sourceUrl": download.source_url.as_str(),
            "fileName": download.file_name,
            "path": download.path.display().to_string(),
            "sizeBytes": download.size_bytes,
            "completedAt": download.completed_at.format(&Rfc3339).unwrap_or_default(),
        },
    })
}

/// Brings Ora forward so the completion toast is not hidden behind the surface window.
pub fn bring_main_window_forward<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(crate::surface::MAIN_WINDOW_LABEL) else {
        ora_warn!(message = "main window unavailable after surface download");
        return;
    };
    if let Err(error) = window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
    {
        // The file is already durable; presentation failures never turn a completed download
        // into a failure.
        ora_warn!(message = "failed to bring Ora forward after surface download", error = %error);
    }
}
