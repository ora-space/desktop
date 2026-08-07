mod downloads;

use std::fs;

use crate::error::CommandError;
use downloads::{DownloadAcceptance, DownloadFinish, DownloadStatus, SkillDownloadCoordinator};
use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_logging::{ora_info, ora_warn};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder,
    webview::{DownloadEvent, NewWindowResponse},
};

const MAIN_WINDOW_LABEL: &str = "main";
const SKILL_MARKETPLACE_STATUS_EVENT: &str = "skill-marketplace://status";
const MARKETPLACE_PROFILE_DIRECTORY: &str = "marketplace-profiles";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum SkillMarketplaceProvider {
    SkillHub,
    HuaweiAgentCenter,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSkillMarketplaceRequest {
    provider: SkillMarketplaceProvider,
}

#[derive(Clone, Copy)]
struct MarketplaceDefinition {
    provider: SkillMarketplaceProvider,
    entry_url: &'static str,
    window_label: &'static str,
    window_title: &'static str,
    profile_directory: &'static str,
    navigation_policy: MarketplaceNavigationPolicy,
}

#[derive(Clone, Copy)]
enum MarketplaceNavigationPolicy {
    ExactHosts(&'static [&'static str]),
    HuaweiInternal,
}

impl SkillMarketplaceProvider {
    /// Resolves the immutable browser boundary for one supported marketplace.
    fn definition(self) -> MarketplaceDefinition {
        match self {
            Self::SkillHub => MarketplaceDefinition {
                provider: self,
                entry_url: "https://www.skillhub.cn",
                window_label: "skillhub-marketplace",
                window_title: "SkillHub",
                profile_directory: "skillhub",
                navigation_policy: MarketplaceNavigationPolicy::ExactHosts(&[
                    "skillhub.cn",
                    "www.skillhub.cn",
                ]),
            },
            Self::HuaweiAgentCenter => MarketplaceDefinition {
                provider: self,
                entry_url: "https://ai.edevops.huawei.com/mcp/projects",
                window_label: "huawei-agent-center",
                window_title: "Huawei Agent Center",
                profile_directory: "huawei-agent-center",
                // The internal SSO redirect inventory is unavailable outside Huawei's network.
                // Keep the first validation inside Huawei's DNS boundary so it can discover the
                // exact hosts without allowing an arbitrary external redirect.
                navigation_policy: MarketplaceNavigationPolicy::HuaweiInternal,
            },
        }
    }
}

impl MarketplaceNavigationPolicy {
    /// Accepts only credential-free HTTPS URLs owned by the selected marketplace boundary.
    fn allows(self, url: &Url) -> bool {
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }

        let Some(host) = url.host_str() else {
            return false;
        };
        match self {
            Self::ExactHosts(hosts) => hosts.contains(&host),
            Self::HuaweiInternal => host == "huawei.com" || host.ends_with(".huawei.com"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum SkillMarketplaceStatus {
    Downloading {
        provider: SkillMarketplaceProvider,
        file_name: String,
    },
    Downloaded {
        provider: SkillMarketplaceProvider,
        file_name: String,
        archive_path: String,
    },
    Failed {
        provider: SkillMarketplaceProvider,
        stage: SkillMarketplaceFailureStage,
        code: String,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum SkillMarketplaceFailureStage {
    Download,
}

/// Opens the requested marketplace or focuses its existing native WebView window.
#[tauri::command]
pub async fn open_skill_marketplace(
    app: AppHandle,
    request: OpenSkillMarketplaceRequest,
) -> Result<(), CommandError> {
    open_or_focus_skill_marketplace(&app, request.provider)
}

/// Reuses one provider-specific window so cookies and login state survive repeated opens.
fn open_or_focus_skill_marketplace<R: Runtime>(
    app: &AppHandle<R>,
    provider: SkillMarketplaceProvider,
) -> Result<(), CommandError> {
    let definition = provider.definition();
    if let Some(window) = app.get_webview_window(definition.window_label) {
        window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
            .map_err(|_| marketplace_window_error())?;
        return Ok(());
    }

    let url = Url::parse(definition.entry_url).map_err(|_| marketplace_window_error())?;
    let app_data_directory = app
        .path()
        .app_data_dir()
        .map_err(|_| download_directory_error())?;
    let downloads = SkillDownloadCoordinator::new(&app_data_directory)
        .map_err(|_| download_directory_error())?;
    let profile_directory = app_data_directory
        .join(MARKETPLACE_PROFILE_DIRECTORY)
        .join(definition.profile_directory);
    fs::create_dir_all(&profile_directory).map_err(|_| marketplace_profile_error())?;

    WebviewWindowBuilder::new(app, definition.window_label, WebviewUrl::External(url))
        .title(definition.window_title)
        .inner_size(1100.0, 760.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .data_directory(profile_directory)
        .on_navigation(move |url| definition.navigation_policy.allows(url))
        .on_new_window(move |url, _features| {
            if definition.navigation_policy.allows(&url) {
                NewWindowResponse::Allow
            } else {
                NewWindowResponse::Deny
            }
        })
        .on_download({
            let app = app.clone();
            move |_webview, event| {
                handle_download_event(&app, &downloads, definition.provider, event)
            }
        })
        .build()
        .map_err(|_| marketplace_window_error())?;

    Ok(())
}

/// Routes the marketplace WebView download lifecycle through Ora-owned ZIP storage.
fn handle_download_event<R: Runtime>(
    app: &AppHandle<R>,
    downloads: &SkillDownloadCoordinator,
    provider: SkillMarketplaceProvider,
    event: DownloadEvent<'_>,
) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => match downloads.request(&url, destination)
        {
            Ok(DownloadAcceptance::Accepted { file_name }) => {
                emit_marketplace_status(
                    app,
                    SkillMarketplaceStatus::Downloading {
                        provider,
                        file_name,
                    },
                );
                ora_info!(
                    message = "marketplace ZIP download started",
                    provider = ?provider,
                    url = %url,
                    destination = %destination.display(),
                );
                true
            }
            Ok(DownloadAcceptance::Rejected) => false,
            Err(error) => {
                emit_download_failure(
                    app,
                    provider,
                    "skill_download_reservation_failed",
                    "Ora could not prepare the marketplace download destination",
                );
                ora_warn!(
                    message = "failed to reserve marketplace ZIP download",
                    provider = ?provider,
                    url = %url,
                    error = %error,
                );
                false
            }
        },
        DownloadEvent::Finished { url, success, .. } => {
            let status = if success {
                DownloadStatus::Succeeded
            } else {
                DownloadStatus::Failed
            };
            match downloads.finish(&url, status) {
                Ok(DownloadFinish::Completed { file_name, path }) => {
                    bring_main_window_forward(app);
                    emit_marketplace_status(
                        app,
                        SkillMarketplaceStatus::Downloaded {
                            provider,
                            file_name,
                            archive_path: path.display().to_string(),
                        },
                    );
                    ora_info!(
                        message = "marketplace ZIP download finished",
                        provider = ?provider,
                        url = %url,
                        result = "completed",
                    );
                    true
                }
                Ok(DownloadFinish::Failed { file_name }) => {
                    emit_download_failure(
                        app,
                        provider,
                        "skill_download_cancelled",
                        &format!("The marketplace download was cancelled: {file_name}"),
                    );
                    true
                }
                Ok(DownloadFinish::Ignored) => true,
                Err(error) => {
                    emit_download_failure(
                        app,
                        provider,
                        "skill_download_finalize_failed",
                        "Ora could not finalize the marketplace ZIP download",
                    );
                    ora_warn!(
                        message = "failed to finalize marketplace ZIP download",
                        provider = ?provider,
                        url = %url,
                        error = %error,
                    );
                    false
                }
            }
        }
        _ => true,
    }
}

/// Brings Ora forward before the transient success toast is emitted behind a marketplace.
fn bring_main_window_forward<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        ora_warn!(message = "main window unavailable after marketplace download");
        return;
    };

    if let Err(error) = window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
    {
        // The archive is already durable, so presentation failures must never turn a completed
        // download into a failure or remove the file that the user requested.
        ora_warn!(
            message = "failed to bring Ora forward after marketplace download",
            error = %error,
        );
    }
}

/// Sends one typed marketplace status to the main window without disrupting the download itself.
fn emit_marketplace_status<R: Runtime>(app: &AppHandle<R>, status: SkillMarketplaceStatus) {
    if let Err(error) = app.emit_to(MAIN_WINDOW_LABEL, SKILL_MARKETPLACE_STATUS_EVENT, status) {
        // Download persistence is the source of truth; a temporarily unavailable UI must not
        // cancel or discard a file that the WebView is already transferring.
        ora_warn!(
            message = "failed to emit marketplace download status",
            error = %error,
        );
    }
}

/// Reports a stable download-stage failure while keeping transport details out of the payload.
fn emit_download_failure<R: Runtime>(
    app: &AppHandle<R>,
    provider: SkillMarketplaceProvider,
    code: &str,
    message: &str,
) {
    emit_marketplace_status(
        app,
        SkillMarketplaceStatus::Failed {
            provider,
            stage: SkillMarketplaceFailureStage::Download,
            code: code.to_owned(),
            message: message.to_owned(),
        },
    );
}

/// Hides platform-specific window failures behind the Desktop command error contract.
fn marketplace_window_error() -> CommandError {
    internal_command_error("failed to open the skill marketplace")
}

/// Reports that Ora could not prepare its persistent marketplace download directory.
fn download_directory_error() -> CommandError {
    internal_command_error("failed to prepare the skill marketplace download directory")
}

/// Reports that Ora could not prepare an isolated persistent browser profile.
fn marketplace_profile_error() -> CommandError {
    internal_command_error("failed to prepare the skill marketplace browser profile")
}

/// Projects an internal marketplace failure through the shared Desktop error contract.
fn internal_command_error(context: &'static str) -> CommandError {
    CommandError::from_backend(BackendError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        context,
    ))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};

    use super::{
        MAIN_WINDOW_LABEL, SkillMarketplaceFailureStage, SkillMarketplaceProvider,
        SkillMarketplaceStatus, bring_main_window_forward, open_or_focus_skill_marketplace,
    };

    /// Verifies both canonical SkillHub hosts remain available over standard HTTPS.
    #[test]
    fn allows_canonical_skillhub_navigation() {
        let policy = SkillMarketplaceProvider::SkillHub
            .definition()
            .navigation_policy;
        assert_eq!(
            [
                "https://skillhub.cn",
                "https://www.skillhub.cn/skills/example?tab=install",
            ]
            .map(parse_url)
            .map(|url| policy.allows(&url)),
            [true, true],
        );
    }

    /// Verifies Huawei SSO navigation stays inside credential-free standard HTTPS URLs.
    #[test]
    fn allows_huawei_internal_navigation() {
        let policy = SkillMarketplaceProvider::HuaweiAgentCenter
            .definition()
            .navigation_policy;
        assert_eq!(
            [
                "https://ai.edevops.huawei.com/mcp/projects",
                "https://sso.huawei.com/login",
                "https://huawei.com/callback",
            ]
            .map(parse_url)
            .map(|url| policy.allows(&url)),
            [true, true, true],
        );
    }

    /// Verifies lookalikes, credentials, custom ports, and insecure schemes are rejected.
    #[test]
    fn rejects_untrusted_marketplace_navigation() {
        let skillhub_policy = SkillMarketplaceProvider::SkillHub
            .definition()
            .navigation_policy;
        let huawei_policy = SkillMarketplaceProvider::HuaweiAgentCenter
            .definition()
            .navigation_policy;
        assert_eq!(
            [
                "http://www.skillhub.cn",
                "https://www.skillhub.cn.evil.example",
                "https://user@www.skillhub.cn",
                "https://www.skillhub.cn:8443",
                "https://example.com",
            ]
            .map(parse_url)
            .map(|url| skillhub_policy.allows(&url)),
            [false, false, false, false, false],
        );
        assert_eq!(
            [
                "http://ai.edevops.huawei.com/mcp/projects",
                "https://huawei.com.evil.example/login",
                "https://user@ai.edevops.huawei.com/mcp/projects",
                "https://sso.huawei.com:8443/login",
                "https://example.com",
            ]
            .map(parse_url)
            .map(|url| huawei_policy.allows(&url)),
            [false, false, false, false, false],
        );
    }

    /// Verifies repeated opens preserve exactly one marketplace window.
    #[test]
    fn reuses_the_existing_marketplace_window() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let definition = SkillMarketplaceProvider::SkillHub.definition();

        open_or_focus_skill_marketplace(&handle, definition.provider)
            .unwrap_or_else(|error| panic!("expected first marketplace open: {error:?}"));
        open_or_focus_skill_marketplace(&handle, definition.provider)
            .unwrap_or_else(|error| panic!("expected marketplace reuse: {error:?}"));

        assert_eq!(
            app.webview_windows()
                .keys()
                .filter(|label| label.as_str() == definition.window_label)
                .count(),
            1,
        );
    }

    /// Verifies a completed download can reveal a hidden main window before status delivery.
    #[test]
    fn brings_the_main_window_forward_for_completed_downloads() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let main_window = WebviewWindowBuilder::new(
            &handle,
            MAIN_WINDOW_LABEL,
            WebviewUrl::App("index.html".into()),
        )
        .visible(false)
        .build()
        .expect("create hidden main window");

        bring_main_window_forward(&handle);

        assert_eq!(main_window.is_visible().expect("read visibility"), true);
    }

    /// Verifies Rust emits the exact tagged payload shape consumed by the platform adapter.
    #[test]
    fn serializes_marketplace_download_statuses() {
        assert_eq!(
            [
                SkillMarketplaceStatus::Downloading {
                    provider: SkillMarketplaceProvider::SkillHub,
                    file_name: "skill.zip".to_owned(),
                },
                SkillMarketplaceStatus::Downloaded {
                    provider: SkillMarketplaceProvider::HuaweiAgentCenter,
                    file_name: "skill.zip".to_owned(),
                    archive_path: "/app-data/skill-downloads/skill.zip".to_owned(),
                },
                SkillMarketplaceStatus::Failed {
                    provider: SkillMarketplaceProvider::HuaweiAgentCenter,
                    stage: SkillMarketplaceFailureStage::Download,
                    code: "skill_download_cancelled".to_owned(),
                    message: "cancelled".to_owned(),
                },
            ]
            .map(|status| serde_json::to_value(status).expect("serialize marketplace status")),
            [
                json!({
                    "status": "downloading",
                    "provider": "skillHub",
                    "fileName": "skill.zip",
                }),
                json!({
                    "status": "downloaded",
                    "provider": "huaweiAgentCenter",
                    "fileName": "skill.zip",
                    "archivePath": "/app-data/skill-downloads/skill.zip",
                }),
                json!({
                    "status": "failed",
                    "provider": "huaweiAgentCenter",
                    "stage": "download",
                    "code": "skill_download_cancelled",
                    "message": "cancelled",
                }),
            ],
        );
    }

    /// Parses one test URL while preserving a useful failure message for malformed fixtures.
    fn parse_url(value: &str) -> Url {
        Url::parse(value).unwrap_or_else(|error| panic!("expected test URL to parse: {error}"))
    }
}
