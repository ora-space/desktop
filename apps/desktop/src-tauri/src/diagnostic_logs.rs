use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use time::{Date, macros::format_description};

const LOG_FILE_PREFIX: &str = "ora.log.";

/// Carries the destination selected by the trusted main Webview's native save dialog.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTodayLogRequest {
    destination: PathBuf,
}

/// Copies the log file currently receiving Ora diagnostics to a user-selected destination.
#[tauri::command]
pub async fn download_today_log(
    app: AppHandle,
    request: DownloadTodayLogRequest,
) -> Result<(), String> {
    let log_directory = super::desktop_data_directory(&app)
        .map_err(|error| format!("failed to resolve the diagnostic log directory: {error}"))?
        .join("logs");
    tauri::async_runtime::spawn_blocking(move || {
        copy_latest_log(&log_directory, &request.destination)
    })
    .await
    .map_err(|error| format!("diagnostic log download task failed: {error}"))?
    .map_err(|error| format!("diagnostic log download failed: {error}"))
}

/// Copies the newest valid daily Ora log, which is the file used by the active rolling writer.
fn copy_latest_log(log_directory: &Path, destination: &Path) -> io::Result<()> {
    let source = latest_log_path(log_directory)?;
    fs::copy(source, destination).map(|_| ())
}

/// Selects only date-suffixed Ora logs and ignores unrelated files in the diagnostics directory.
fn latest_log_path(log_directory: &Path) -> io::Result<PathBuf> {
    let mut latest = None;
    for entry in fs::read_dir(log_directory)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(date) = file_name.strip_prefix(LOG_FILE_PREFIX).and_then(|suffix| {
            Date::parse(suffix, &format_description!("[year]-[month]-[day]")).ok()
        }) else {
            continue;
        };
        if latest
            .as_ref()
            .is_none_or(|(latest_date, _)| date > *latest_date)
        {
            latest = Some((date, path));
        }
    }

    latest.map(|(_, path)| path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no daily Ora diagnostic log is available",
        )
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{copy_latest_log, latest_log_path};

    /// The active daily log wins over older logs and unrelated files.
    #[test]
    fn copies_latest_daily_log() {
        let temp = tempfile::tempdir().unwrap();
        let logs = temp.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        std::fs::write(logs.join("ora.log.2026-09-02"), "old\n").unwrap();
        std::fs::write(logs.join("ora.log.2026-09-03"), "current\n").unwrap();
        std::fs::write(logs.join("unrelated.log"), "ignore\n").unwrap();
        let destination = temp.path().join("downloaded.log");

        copy_latest_log(&logs, &destination).unwrap();

        assert_eq!(std::fs::read_to_string(destination).unwrap(), "current\n");
    }

    /// Malformed Ora-like names cannot displace a valid dated log.
    #[test]
    fn ignores_malformed_log_names() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("ora.log.2026-09-03"), "valid").unwrap();
        std::fs::write(temp.path().join("ora.log.tomorrow"), "invalid").unwrap();

        assert_eq!(
            latest_log_path(temp.path()).unwrap(),
            temp.path().join("ora.log.2026-09-03")
        );
    }

    /// A missing daily log reports NotFound so the frontend can surface a download failure.
    #[test]
    fn rejects_directory_without_daily_logs() {
        let temp = tempfile::tempdir().unwrap();

        assert_eq!(
            latest_log_path(temp.path()).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
