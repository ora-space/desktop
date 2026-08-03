use crate::error::CommandError;
use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};
use tauri::{AppHandle, Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder};

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
    WebviewWindowBuilder::new(app, SKILLHUB_WINDOW_LABEL, WebviewUrl::External(url))
        .title("SkillHub")
        .inner_size(1100.0, 760.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .on_navigation(is_skillhub_navigation_allowed)
        .build()
        .map_err(|_| marketplace_window_error())?;

    Ok(())
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
    CommandError::from_backend(BackendError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        "failed to open the SkillHub marketplace",
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
