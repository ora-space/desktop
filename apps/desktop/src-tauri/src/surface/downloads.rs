//! The webview-plugin download pipeline: browser transfer landing, rule selection, the managed
//! state machine, and the frontend events that drive user choice.
//!
//! A webview plugin has no process; the host owns the whole download. Bytes land in the plugin's
//! `webview/downloads/` directory through `ora-utils::fs` safe-naming, the disposition is chosen
//! from the plugin's manifest rules against the page URL frozen at request time, and the outcome
//! is either an automatic host action or a prompt to the trusted main webview.

use crate::surface::MAIN_WINDOW_LABEL;
use crate::surface::download_actions::DownloadActionHost;
use crate::surface::effects::emit_event;
use crate::surface::gateway::SurfacePluginGateway;
use crate::surface::hooks::DownloadSink;
use ora_logging::{ora_info, ora_warn};
use ora_plugin_manifest::DownloadAction;
use ora_surface::{
    DownloadDecision, DownloadId, DownloadIntent, ManagedDownload, SurfaceEvent, SurfaceRecord,
    SurfaceRegistry, SurfaceSource, WebviewLabel, select_disposition,
};
use ora_utils::fs::{next_available_file_name, sanitize_file_name};
use semver::Version;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager, Runtime, Url, UserAttentionType};

/// Host-written child of the plugin data directory that holds downloaded artifacts.
const DOWNLOADS_DIRECTORY: &str = "webview/downloads";
/// Stem used when neither the suggested name nor the URL yields a usable one.
const FALLBACK_STEM: &str = "download";
/// Suffix of an in-flight file; a completed file never carries it.
const PART_SUFFIX: &str = ".part";

/// A landed artifact the host owns; only the `download_id` and this struct cross module lines.
#[derive(Clone, Debug)]
pub struct StagedArtifact {
    pub path: PathBuf,
    pub file_name: String,
}

/// One tracked download: its state machine plus the on-disk paths the host owns.
struct TrackedDownload {
    managed: ManagedDownload,
    /// `.part` staging path the engine writes through. `None` for `blob:` sources, which the
    /// engine lands at its own destination: re-targeting such a transfer aborts it.
    part_path: Option<PathBuf>,
    final_path: PathBuf,
}

/// Routes download events of every remote-site webview and owns the managed downloads.
///
/// The destination is decided solely by the webview label resolved through the registry, so a
/// remote page can never steer a file into another plugin's directory.
pub struct DownloadDispatcher<G, R: Runtime> {
    registry: Arc<SurfaceRegistry>,
    gateway: G,
    app: AppHandle<R>,
    next_id: AtomicU64,
    /// Runs host actions of automatic dispositions; installed once the backend exists. A dyn
    /// slot (like the lifecycle's `SurfaceCloser`) because the service is built before the
    /// desktop state that owns the backend.
    action_host: OnceLock<Arc<dyn DownloadActionHost>>,
    /// Keyed by `download_id`; also indexed by `(label, url)` for the finish callback. Shared
    /// with the blocking tasks that settle automatic actions after the dispatcher call returned.
    tracked: Arc<Mutex<HashMap<u64, TrackedDownload>>>,
    /// FIFO queue per `(label, url)`: the browser engine reports finishes without a native
    /// download id, and the same page may download the same URL concurrently, so completions
    /// are matched to requests in start order instead of overwriting a single slot.
    by_url: Mutex<HashMap<(String, String), VecDeque<u64>>>,
}

/// Why a resolve/discard request could not be served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// No download with that id is awaiting a choice.
    UnknownDownload,
    /// The chosen action was not one this download was frozen with.
    ActionNotAllowed,
}

impl<G: SurfacePluginGateway, R: Runtime> DownloadDispatcher<G, R> {
    /// Builds an empty dispatcher bound to one gateway, app handle, and registry.
    pub fn new(registry: Arc<SurfaceRegistry>, gateway: G, app: AppHandle<R>) -> Self {
        Self {
            registry,
            gateway,
            app,
            next_id: AtomicU64::new(1),
            action_host: OnceLock::new(),
            tracked: Arc::new(Mutex::new(HashMap::new())),
            by_url: Mutex::new(HashMap::new()),
        }
    }

    /// Installs the executor of automatic download actions; later installs are ignored.
    pub fn install_action_host(&self, host: Arc<dyn DownloadActionHost>) {
        let _ = self.action_host.set(host);
    }

    /// Selects a disposition, reserves a `.part` path, and redirects the browser to it.
    ///
    /// A rejected download returns `false`, which denies the browser transfer; otherwise the
    /// destination is rewritten to the reserved path and a `Staging` download is tracked. A
    /// `blob:` URL is the exception: the engine keeps the transfer at its own default
    /// destination (see the branch below).
    /// `pub(super)` so the host tests can drive the pipeline without constructing Tauri's
    /// non-exhaustive `DownloadEvent`.
    pub(super) fn requested(
        &self,
        record: &SurfaceRecord,
        page_url: Option<Url>,
        url: &Url,
        destination: &mut PathBuf,
    ) -> bool {
        let SurfaceSource::RemoteSite(site) = &record.definition.source else {
            return false;
        };
        let Some(page_url) = page_url else {
            ora_warn!(message = "download without an initiating page url is refused", url = %url);
            return false;
        };
        let decision = select_disposition(&site.download_policy, &page_url);
        if matches!(decision, DownloadDecision::Reject) {
            ora_info!(message = "webview download rejected by policy", plugin_id = %record.definition.plugin_id, url = %url);
            return false;
        }
        // A `blob:` URL carries bytes the page built in memory rather than a network resource.
        // WebView2 aborts such a transfer almost immediately when its write target is re-opened
        // through the download manager — the observed failure was an instant "transfer failed"
        // — so blob downloads stay at the engine's own destination and the host only tracks the
        // path the import flow will read from.
        let blob_source = url.scheme() == "blob";
        let suggested = destination
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .or_else(|| {
                url.path_segments()
                    .and_then(Iterator::last)
                    .map(str::to_owned)
            })
            .unwrap_or_default();

        // `None` part path: the engine owns the landing file, so there is nothing to stage,
        // promote, or reserve in the plugin directory.
        let (part_path, final_path) = if blob_source {
            (None, destination.clone())
        } else {
            let directory = match self.gateway.data_directory(&record.definition.plugin_id) {
                Ok(directory) => directory.join(DOWNLOADS_DIRECTORY),
                Err(error) => {
                    ora_warn!(message = "plugin data directory unavailable for download", error = %error);
                    return false;
                }
            };
            if let Err(error) = std::fs::create_dir_all(&directory) {
                ora_warn!(message = "download directory could not be created", error = %error);
                return false;
            }
            let file_name = sanitize_file_name(&suggested, FALLBACK_STEM);
            // Reserve a unique final name against on-disk files and in-flight reservations, then
            // use its `.part` sibling as the transfer target.
            let held = self.reserved_final_paths();
            let final_path = next_available_file_name(&directory, &file_name, |candidate| {
                held.iter().any(|reserved| reserved == candidate)
            });
            let part_path = final_path.with_extension(format!(
                "{}{PART_SUFFIX}",
                final_path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
            ));
            (Some(part_path), final_path)
        };
        let file_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(suggested.as_str())
            .to_owned();

        let download_id = DownloadId::new(self.next_id.fetch_add(1, Ordering::Relaxed));
        let intent = DownloadIntent {
            download_id,
            instance: record.instance,
            plugin_id: record.definition.plugin_id.clone(),
            exact_version: exact_version(record),
            initiating_page_url: page_url,
            download_url: url.to_string(),
            suggested_file_name: suggested,
            disposition: disposition_for(&decision),
        };
        if let Some(part_path) = &part_path {
            *destination = part_path.clone();
        }
        self.tracked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                download_id.value(),
                TrackedDownload {
                    managed: ManagedDownload::staging(intent),
                    part_path,
                    final_path,
                },
            );
        self.by_url
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entry((record.label.as_str().to_owned(), url.to_string()))
            .or_default()
            .push_back(download_id.value());
        // The accept is silent in the UI when a transfer later fails, so this line is the only
        // record of which URL and file the engine was redirected to.
        ora_info!(
            message = "webview download accepted",
            plugin_id = %record.definition.plugin_id,
            download_id = download_id.value(),
            url = %url,
            file_name = %file_name,
        );
        emit_event(
            &self.app,
            &SurfaceEvent::DownloadStarted {
                instance: record.instance.value(),
                plugin_id: record.definition.plugin_id.to_string(),
                download_id: download_id.value(),
                file_name,
            },
        );
        true
    }

    /// Promotes or discards the `.part` file and either runs the auto action or prompts.
    /// `pub(super)` for the same reason as [`Self::requested`].
    pub(super) fn finished(&self, record: &SurfaceRecord, url: &Url, success: bool) {
        let key = (record.label.as_str().to_owned(), url.to_string());
        let download_id = {
            let mut by_url = self
                .by_url
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(queue) = by_url.get_mut(&key) else {
                return;
            };
            let Some(download_id) = queue.pop_front() else {
                return;
            };
            if queue.is_empty() {
                by_url.remove(&key);
            }
            download_id
        };
        let mut tracked = self
            .tracked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(entry) = tracked.get_mut(&download_id) else {
            return;
        };
        if !success {
            if let Some(part_path) = &entry.part_path {
                let _ = std::fs::remove_file(part_path);
            }
            entry.managed = entry
                .managed
                .clone()
                .fail("the browser engine reported a failed transfer")
                .unwrap_or_else(|_| entry.managed.clone());
            emit_download_failed(&self.app, record, download_id, entry, "transfer failed");
            tracked.remove(&download_id);
            return;
        }
        // Promote `.part` to its reserved final name; a name taken by an outside file since the
        // reservation is re-resolved rather than overwritten. A `blob:` download skipped the
        // staging area and already sits at its final path, so there is nothing to promote.
        if let Some(part_path) = &entry.part_path {
            if let Some(parent) = entry.final_path.parent() {
                let file_name = entry
                    .final_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(FALLBACK_STEM)
                    .to_owned();
                let held = Vec::new();
                let promoted = next_available_file_name(parent, &file_name, |candidate| {
                    held.iter()
                        .any(|reserved: &PathBuf| reserved.as_path() == candidate)
                });
                entry.final_path = promoted;
            }
            if let Err(error) = std::fs::rename(part_path, &entry.final_path) {
                ora_warn!(message = "download could not be promoted", error = %error);
                let _ = std::fs::remove_file(part_path);
                emit_download_failed(
                    &self.app,
                    record,
                    download_id,
                    entry,
                    "could not be finalized",
                );
                tracked.remove(&download_id);
                return;
            }
        }
        match entry.managed.clone().landed() {
            Ok(landed) => {
                entry.managed = landed;
                match &entry.managed {
                    ManagedDownload::AwaitingChoice {
                        allowed_actions, ..
                    } => {
                        // The choice dialog lives in the main window and needs an answer, so the
                        // window is raised above a surface window that may cover it.
                        nudge_main_window(&self.app, true);
                        emit_event(
                            &self.app,
                            &SurfaceEvent::DownloadChoice {
                                instance: record.instance.value(),
                                plugin_id: record.definition.plugin_id.to_string(),
                                download_id,
                                page_origin: page_origin(entry),
                                file_name: file_name_of(entry),
                                size_bytes: file_size(&entry.final_path),
                                actions: allowed_actions
                                    .iter()
                                    .map(|action| action.as_str().to_owned())
                                    .collect(),
                            },
                        );
                    }
                    ManagedDownload::Processing { action, .. } => {
                        // Automatic disposition: the choice was skipped, but the action still has
                        // to run before any success is reported. It runs off this webview event
                        // thread; `downloadCompleted` follows only from the finished action.
                        let action = *action;
                        let artifact = StagedArtifact {
                            path: entry.final_path.clone(),
                            file_name: file_name_of(entry),
                        };
                        self.run_auto_action(record, download_id, action, artifact);
                    }
                    ManagedDownload::Staging { .. } | ManagedDownload::Settled { .. } => {}
                }
            }
            Err(_) => {
                ora_warn!(
                    message = "download entered an invalid state while landing",
                    plugin_id = %record.definition.plugin_id,
                    download_id,
                );
                tracked.remove(&download_id);
            }
        }
    }

    /// Takes a download awaiting a choice into processing for one action and returns its artifact.
    ///
    /// The linearization point: a second call for the same download after it is processing is
    /// refused, so an action can never consume the file twice.
    pub fn take_for_action(
        &self,
        download_id: u64,
        action: DownloadAction,
    ) -> Result<StagedArtifact, ResolveError> {
        let mut tracked = self
            .tracked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = tracked
            .get_mut(&download_id)
            .ok_or(ResolveError::UnknownDownload)?;
        let processing = entry
            .managed
            .clone()
            .choose(action)
            .map_err(|_| ResolveError::ActionNotAllowed)?;
        entry.managed = processing;
        Ok(StagedArtifact {
            path: entry.final_path.clone(),
            file_name: file_name_of(entry),
        })
    }

    /// Settles a processing download, dropping its tracking entry.
    pub fn settle(&self, download_id: u64, failure: Option<String>) {
        let mut tracked = self
            .tracked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(_entry) = tracked.remove(&download_id)
            && let Some(reason) = failure
        {
            ora_warn!(message = "download action failed", download_id, reason = %reason);
        }
    }

    /// Discards a download the user dismissed, removing the landed file.
    pub fn discard(&self, download_id: u64) -> Result<(), ResolveError> {
        let mut tracked = self
            .tracked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = tracked
            .remove(&download_id)
            .ok_or(ResolveError::UnknownDownload)?;
        let _ = std::fs::remove_file(&entry.final_path);
        Ok(())
    }

    /// Runs the host action of an automatic disposition in a blocking task and settles the
    /// download with `downloadCompleted` only after the action succeeded (or `downloadFailed`,
    /// removing the landed file, when it did not).
    fn run_auto_action(
        &self,
        record: &SurfaceRecord,
        download_id: u64,
        action: DownloadAction,
        artifact: StagedArtifact,
    ) {
        let app = self.app.clone();
        let tracked = self.tracked.clone();
        let instance = record.instance.value();
        let plugin_id = record.definition.plugin_id.to_string();
        let Some(host) = self.action_host.get().cloned() else {
            // Installed during desktop setup; missing it is a wiring bug, not a user condition.
            ora_warn!(
                message = "automatic download action has no installed host",
                download_id
            );
            settle_auto_action(
                &app,
                &tracked,
                instance,
                &plugin_id,
                download_id,
                action,
                &artifact,
                Err("the download action host is not available".to_owned()),
            );
            return;
        };
        tauri::async_runtime::spawn_blocking(move || {
            let outcome = execute_auto_action(host.as_ref(), action, &artifact);
            settle_auto_action(
                &app,
                &tracked,
                instance,
                &plugin_id,
                download_id,
                action,
                &artifact,
                outcome,
            );
        });
    }

    /// Every reserved final path (used to keep concurrent reservations distinct).
    ///
    /// Engine-landed `blob:` downloads write outside the plugin directory, so their paths never
    /// participate in the reservation namespace.
    fn reserved_final_paths(&self) -> Vec<PathBuf> {
        self.tracked
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .filter(|entry| entry.part_path.is_some())
            .map(|entry| entry.final_path.clone())
            .collect()
    }
}

impl<G: SurfacePluginGateway, R: Runtime> DownloadSink<R> for DownloadDispatcher<G, R> {
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
                self.finished(&record, &url, success);
                true
            }
            // `DownloadEvent` is `#[non_exhaustive]`; unknown future events must not block.
            _ => true,
        }
    }
}

/// Executes one automatic action against a landed artifact; `Ok` carries the follow-up the
/// frontend needs (the prepared skill-import session id), if any.
fn execute_auto_action(
    host: &dyn DownloadActionHost,
    action: DownloadAction,
    artifact: &StagedArtifact,
) -> Result<Option<String>, String> {
    match action {
        DownloadAction::ImportSkill => host
            .prepare_skill_import(&artifact.path, &artifact.file_name)
            .map(Some)
            .map_err(|error| error.to_string()),
        // Manifest validation refuses `auto = "save_as"`; kept total so a validation regression
        // fails loudly here instead of reporting a success that never happened.
        DownloadAction::SaveAs => Err(
            "save_as requires a user-chosen destination and cannot run automatically".to_owned(),
        ),
    }
}

/// Drops the tracking entry of an automatic action and reports its real outcome; a failed action
/// also removes the landed file so nothing unreferenced accumulates in the downloads directory.
#[allow(clippy::too_many_arguments)]
fn settle_auto_action<R: Runtime>(
    app: &AppHandle<R>,
    tracked: &Mutex<HashMap<u64, TrackedDownload>>,
    instance: u64,
    plugin_id: &str,
    download_id: u64,
    action: DownloadAction,
    artifact: &StagedArtifact,
    outcome: Result<Option<String>, String>,
) {
    tracked
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(&download_id);
    match outcome {
        Ok(import_session_id) => {
            // An import session opens the review dialog in the main window and needs an answer;
            // a bare completion only toasts. Either way a surface window may be covering it.
            nudge_main_window(app, import_session_id.is_some());
            emit_event(
                app,
                &SurfaceEvent::DownloadCompleted {
                    instance,
                    plugin_id: plugin_id.to_owned(),
                    download_id,
                    file_name: artifact.file_name.clone(),
                    action: action.as_str().to_owned(),
                    import_session_id,
                },
            );
        }
        Err(reason) => {
            let _ = std::fs::remove_file(&artifact.path);
            ora_warn!(message = "automatic download action failed", download_id, reason = %reason);
            nudge_main_window(app, false);
            emit_event(
                app,
                &SurfaceEvent::DownloadFailed {
                    instance,
                    plugin_id: plugin_id.to_owned(),
                    download_id,
                    file_name: artifact.file_name.clone(),
                    reason,
                },
            );
        }
    }
}

/// Draws the user's attention to download feedback a surface window may be covering.
///
/// Download feedback renders in the main webview's DOM, while a surface lives in a native view or
/// separate window that always paints above DOM. Prompts need an answer and steal focus; plain
/// toasts only flash the taskbar so they never interrupt typing elsewhere.
fn nudge_main_window<R: Runtime>(app: &AppHandle<R>, steal_focus: bool) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _ = window.request_user_attention(Some(UserAttentionType::Informational));
    if steal_focus {
        let _ = window.set_focus();
    }
}

/// Emits a `downloadFailed` event for one tracked download.
///
/// The failure toast is the only user-visible signal for a failed transfer, and the engine does
/// not report why, so this log line is what makes an otherwise silent failure diagnosable.
fn emit_download_failed<R: Runtime>(
    app: &AppHandle<R>,
    record: &SurfaceRecord,
    download_id: u64,
    entry: &TrackedDownload,
    reason: &str,
) {
    ora_warn!(
        message = "webview download failed",
        plugin_id = %record.definition.plugin_id,
        download_id,
        file_name = %file_name_of(entry),
        reason,
    );
    nudge_main_window(app, false);
    emit_event(
        app,
        &SurfaceEvent::DownloadFailed {
            instance: record.instance.value(),
            plugin_id: record.definition.plugin_id.to_string(),
            download_id,
            file_name: file_name_of(entry),
            reason: reason.to_owned(),
        },
    );
}

/// The final file name of a tracked download.
fn file_name_of(entry: &TrackedDownload) -> String {
    entry
        .final_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(FALLBACK_STEM)
        .to_owned()
}

/// The origin of the page that initiated a download, for the choice prompt.
fn page_origin(entry: &TrackedDownload) -> String {
    entry
        .managed
        .intent()
        .initiating_page_url
        .origin()
        .ascii_serialization()
}

/// The byte size of a landed file, or zero if it cannot be read.
fn file_size(path: &PathBuf) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// The exact installed version bound to a record's plugin, defaulting to `0.0.0` if unknown.
fn exact_version(_record: &SurfaceRecord) -> Version {
    // The registry record does not carry the version; the intent keeps it only for logging and
    // is never used to route, so a placeholder is acceptable until the record carries it.
    Version::new(0, 0, 0)
}

/// Reconstructs a manifest disposition from the host decision so the state machine can carry it.
fn disposition_for(decision: &DownloadDecision) -> ora_plugin_manifest::DownloadDisposition {
    use ora_plugin_manifest::DownloadDisposition;
    match decision {
        DownloadDecision::Reject => DownloadDisposition::Reject,
        DownloadDecision::Auto(action) => DownloadDisposition::Auto { action: *action },
        DownloadDecision::Prompt(actions) => DownloadDisposition::Prompt {
            actions: actions.clone(),
        },
    }
}
