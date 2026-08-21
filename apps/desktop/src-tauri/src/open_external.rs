use crate::error::CommandError;
use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::{EmptyErrorParams, PublicError};
use serde::Deserialize;
use tracing::Instrument;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenExternalUrlRequest {
    pub url: String,
}

/// Opens an http(s) or mailto URL in the host browser so prompt-box links leave the webview.
#[tauri::command]
pub async fn open_external_url(request: OpenExternalUrlRequest) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start("open_external_url", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    let url = request.url;
    // Windows ShellExecuteW returns without waiting for the browser, so the
    // first click must not hop through spawn_blocking (cold thread pool + the
    // old cmd.exe path is what made that press feel frozen). Other hosts still
    // spawn because `open` / `xdg-open` can block.
    let result = {
        #[cfg(target_os = "windows")]
        {
            request_span.in_scope(|| open_external_url_blocking(&url))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let blocking_span = request_span.clone();
            match tauri::async_runtime::spawn_blocking(move || {
                blocking_span.in_scope(|| open_external_url_blocking(&url))
            })
            .await
            {
                Ok(result) => result,
                Err(source) => Err(BackendError::internal(
                    "Desktop command execution failed",
                    source,
                )),
            }
        }
    };
    async move {
        match result {
            Ok(()) => {
                lifecycle.complete_success();
                Ok(())
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

/// Rejects javascript/file/data URLs so the prompt box cannot launch local handlers.
fn is_browser_url(raw: &str) -> bool {
    if raw.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = raw.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

/// Client sent a scheme Desktop will not open (javascript/file/data, etc.).
fn open_external_url_scheme_error(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ora_backend::ErrorClassification::InvalidRequest,
        PublicError::InvalidRequest(EmptyErrorParams {}),
        "URL scheme is not allowed",
        source,
    )
}

/// Host OS refused to launch the browser; not a problem with the URL itself.
fn open_external_url_os_error(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::internal("failed to open the requested URL", source)
}

/// Launches the OS browser for one validated URL through the Windows shell.
///
/// `cmd /C start` wraps this same API behind a new `cmd.exe`. That extra process
/// is what made the first prompt-box click feel frozen: Defender scans the
/// binary, then `start` looks up the protocol handler. Calling `ShellExecuteW`
/// in-process skips that.
#[cfg(target_os = "windows")]
fn open_external_url_blocking(url: &str) -> Result<(), BackendError> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    if !is_browser_url(url) {
        return Err(open_external_url_scheme_error(std::io::Error::other(
            "URL scheme is not allowed",
        )));
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut core::ffi::c_void,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> isize;
    }

    const SW_SHOWNORMAL: i32 = 1;
    let operation: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(url).encode_wide().chain(Some(0)).collect();
    // SAFETY: `operation` and `file` are live null-terminated UTF-16 buffers for
    // the duration of this call; hwnd/params/dir are optional per ShellExecuteW.
    // The call returns after handing the URL to the registered handler; values
    // 0..=32 are documented error codes rather than a real HINSTANCE.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result <= 32 {
        return Err(open_external_url_os_error(std::io::Error::other(format!(
            "ShellExecuteW failed with code {result}"
        ))));
    }
    Ok(())
}

/// Launches the OS browser through macOS `open`.
#[cfg(target_os = "macos")]
fn open_external_url_blocking(url: &str) -> Result<(), BackendError> {
    use std::process::Command;
    if !is_browser_url(url) {
        return Err(open_external_url_scheme_error(std::io::Error::other(
            "URL scheme is not allowed",
        )));
    }
    let status = Command::new("open")
        .arg(url)
        .status()
        .map_err(open_external_url_os_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(open_external_url_os_error(std::io::Error::other(format!(
            "open command exited with {status}"
        ))))
    }
}

/// Linux Desktop is not a shipping host; keep the command defined for completeness.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_external_url_blocking(url: &str) -> Result<(), BackendError> {
    use std::process::Command;
    if !is_browser_url(url) {
        return Err(open_external_url_scheme_error(std::io::Error::other(
            "URL scheme is not allowed",
        )));
    }
    Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(open_external_url_os_error)
}

#[cfg(test)]
mod tests {
    use super::is_browser_url;

    #[test]
    fn accepts_http_https_and_mailto() {
        assert!(is_browser_url("https://example.com/path"));
        assert!(is_browser_url("http://example.com"));
        assert!(is_browser_url("mailto:dev@example.com"));
        assert!(is_browser_url("HTTPS://EXAMPLE.COM"));
    }

    #[test]
    fn rejects_local_and_script_schemes() {
        assert!(!is_browser_url("javascript:alert(1)"));
        assert!(!is_browser_url("file:///C:/secret"));
        assert!(!is_browser_url("data:text/html,hi"));
        assert!(!is_browser_url("https://example.com/path with space"));
        assert!(!is_browser_url(""));
    }
}
