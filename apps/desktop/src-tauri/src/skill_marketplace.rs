mod downloads;

use crate::error::CommandError;
use downloads::{DownloadAcceptance, DownloadStatus, SkillDownloadCoordinator};
use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_logging::{ora_info, ora_warn};
use tauri::{
    AppHandle, Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder, webview::DownloadEvent,
};

const SKILLHUB_URL: &str = "https://www.skillhub.cn";
const SKILLHUB_WINDOW_LABEL: &str = "skillhub-marketplace";

/// Opens the SkillHub marketplace or focuses the existing marketplace window.
#[tauri::command]
pub async fn open_skill_marketplace(app: AppHandle) -> Result<(), CommandError> {
    open_or_focus_skill_marketplace(&app)
}

/// Reuses a single window so navigation, cookies, and login state survive repeated opens.
fn open_or_focus_skill_marketplace<R: Runtime>(app: &AppHandle<R>) -> Result<(), CommandError> {
    if let Some(window) = app.get_webview_window(SKILLHUB_WINDOW_LABEL) {
        window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
            .map_err(|_| marketplace_window_error())?;
        return Ok(());
    }

    let url = Url::parse(SKILLHUB_URL).map_err(|_| marketplace_window_error())?;
    let app_data_directory = app
        .path()
        .app_data_dir()
        .map_err(|_| download_directory_error())?;
    let downloads = SkillDownloadCoordinator::new(&app_data_directory)
        .map_err(|_| download_directory_error())?;
    WebviewWindowBuilder::new(app, SKILLHUB_WINDOW_LABEL, WebviewUrl::External(url))
        .title("SkillHub")
        .inner_size(1100.0, 760.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .on_navigation(is_skillhub_navigation_allowed)
        .on_download(move |_webview, event| handle_download_event(&downloads, event))
        .build()
        .map_err(|_| marketplace_window_error())?;

    Ok(())
}

/// Routes the marketplace WebView download lifecycle through Ora-owned ZIP storage.
fn handle_download_event(downloads: &SkillDownloadCoordinator, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => match downloads.request(&url, destination)
        {
            Ok(DownloadAcceptance::Accepted) => {
                ora_info!(
                    message = "SkillHub ZIP download started",
                    url = %url,
                    destination = %destination.display(),
                );
                true
            }
            Ok(DownloadAcceptance::Rejected) => false,
            Err(error) => {
                ora_warn!(
                    message = "failed to reserve SkillHub ZIP download",
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
                Ok(result) => {
                    ora_info!(
                        message = "SkillHub ZIP download finished",
                        url = %url,
                        result = ?result,
                    );
                    true
                }
                Err(error) => {
                    ora_warn!(
                        message = "failed to finalize SkillHub ZIP download",
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

/// Allows top-level navigation only to canonical SkillHub hosts over standard HTTPS.
fn is_skillhub_navigation_allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host_str(), Some("skillhub.cn" | "www.skillhub.cn"))
}

/// Hides platform-specific window failures behind the Desktop command error contract.
fn marketplace_window_error() -> CommandError {
    internal_command_error("failed to open the SkillHub marketplace")
}

/// Reports that Ora could not prepare its persistent SkillHub download directory.
fn download_directory_error() -> CommandError {
    internal_command_error("failed to prepare the SkillHub download directory")
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
    use tauri::{Manager, Url};

    use super::{
        SKILLHUB_WINDOW_LABEL, is_skillhub_navigation_allowed, open_or_focus_skill_marketplace,
    };

    /// Verifies both canonical SkillHub hosts remain available over standard HTTPS.
    #[test]
    fn allows_canonical_skillhub_navigation() {
        assert_eq!(
            [
                "https://skillhub.cn",
                "https://www.skillhub.cn/skills/example?tab=install",
            ]
            .map(parse_url)
            .map(|url| is_skillhub_navigation_allowed(&url)),
            [true, true],
        );
    }

    /// Verifies lookalike hosts, credentials, custom ports, and insecure schemes are rejected.
    #[test]
    fn rejects_untrusted_marketplace_navigation() {
        assert_eq!(
            [
                "http://www.skillhub.cn",
                "https://www.skillhub.cn.evil.example",
                "https://user@www.skillhub.cn",
                "https://www.skillhub.cn:8443",
                "https://example.com",
            ]
            .map(parse_url)
            .map(|url| is_skillhub_navigation_allowed(&url)),
            [false, false, false, false, false],
        );
    }

    /// Verifies repeated opens preserve exactly one marketplace window.
    #[test]
    fn reuses_the_existing_marketplace_window() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();

        open_or_focus_skill_marketplace(&handle)
            .unwrap_or_else(|error| panic!("expected first marketplace open: {error:?}"));
        open_or_focus_skill_marketplace(&handle)
            .unwrap_or_else(|error| panic!("expected marketplace reuse: {error:?}"));

        assert_eq!(
            app.webview_windows()
                .keys()
                .filter(|label| label.as_str() == SKILLHUB_WINDOW_LABEL)
                .count(),
            1,
        );
    }

    /// Parses one test URL while preserving a useful failure message for malformed fixtures.
    fn parse_url(value: &str) -> Url {
        Url::parse(value).unwrap_or_else(|error| panic!("expected test URL to parse: {error}"))
    }
}
