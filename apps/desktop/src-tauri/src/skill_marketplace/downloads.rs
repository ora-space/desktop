use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use tauri::Url;

const DOWNLOAD_DIRECTORY_NAME: &str = "skill-downloads";
const DEFAULT_ZIP_FILE_NAME: &str = "skill.zip";
const MAX_FILE_STEM_BYTES: usize = 120;

/// Coordinates collision-free temporary and final paths for marketplace downloads.
pub(super) struct SkillDownloadCoordinator {
    directory: PathBuf,
    active: Mutex<HashMap<String, DownloadPaths>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DownloadPaths {
    final_path: PathBuf,
    part_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DownloadAcceptance {
    Accepted { file_name: String },
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DownloadStatus {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DownloadFinish {
    Completed { file_name: String, path: PathBuf },
    Failed { file_name: String },
    Ignored,
}

impl SkillDownloadCoordinator {
    /// Creates the application-owned download directory before the WebView can request a file.
    pub(super) fn new(app_data_directory: &Path) -> io::Result<Self> {
        let directory = app_data_directory.join(DOWNLOAD_DIRECTORY_NAME);
        fs::create_dir_all(&directory)?;
        Ok(Self {
            directory,
            active: Mutex::new(HashMap::new()),
        })
    }

    /// Returns the persistent directory used by completed archives in unit tests.
    #[cfg(test)]
    fn directory(&self) -> &Path {
        &self.directory
    }

    /// Reserves a unique `.part` destination for a supported ZIP download.
    pub(super) fn request(
        &self,
        url: &Url,
        suggested_destination: &mut PathBuf,
    ) -> io::Result<DownloadAcceptance> {
        let Some(file_name) = zip_file_name(url, suggested_destination) else {
            return Ok(DownloadAcceptance::Rejected);
        };

        // The directory may be removed while Ora is running, so recreate it at the last responsible moment.
        fs::create_dir_all(&self.directory)?;
        let mut active = self.lock_active()?;
        let key = url.to_string();
        if active.contains_key(&key) {
            return Ok(DownloadAcceptance::Rejected);
        }
        let paths = available_paths(&self.directory, &file_name, &active);
        *suggested_destination = paths.part_path.clone();
        let file_name = final_file_name(&paths.final_path);
        active.insert(key, paths);
        Ok(DownloadAcceptance::Accepted { file_name })
    }

    /// Promotes a successful `.part` file or removes it after failure or cancellation.
    pub(super) fn finish(&self, url: &Url, status: DownloadStatus) -> io::Result<DownloadFinish> {
        let Some(paths) = self.take_paths(url, status)? else {
            return Ok(DownloadFinish::Ignored);
        };

        if status == DownloadStatus::Failed {
            remove_file_if_present(&paths.part_path)?;
            return Ok(DownloadFinish::Failed {
                file_name: final_file_name(&paths.final_path),
            });
        }

        if let Err(error) = fs::rename(&paths.part_path, &paths.final_path) {
            // A failed handoff must not leave an archive-shaped partial file for later installers.
            let _ = remove_file_if_present(&paths.part_path);
            return Err(error);
        }

        let file_name = final_file_name(&paths.final_path);
        Ok(DownloadFinish::Completed {
            file_name,
            path: paths.final_path,
        })
    }

    /// Removes the reservation and rechecks its final name before a successful promotion.
    fn take_paths(&self, url: &Url, status: DownloadStatus) -> io::Result<Option<DownloadPaths>> {
        let mut active = self.lock_active()?;
        let key = url.to_string();
        let Some(mut paths) = active.remove(&key) else {
            return Ok(None);
        };

        // Another process may create the reserved final name during the download. Re-reserving
        // here prevents Ora from intentionally replacing that archive during finalization.
        if status == DownloadStatus::Succeeded && paths.final_path.exists() {
            let file_name = paths
                .final_path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or(DEFAULT_ZIP_FILE_NAME);
            paths.final_path = available_paths(&self.directory, file_name, &active).final_path;
        }
        Ok(Some(paths))
    }

    /// Converts a poisoned state lock into an I/O error so the callback can reject safely.
    fn lock_active(&self) -> io::Result<MutexGuard<'_, HashMap<String, DownloadPaths>>> {
        self.active
            .lock()
            .map_err(|_| io::Error::other("marketplace download state lock is poisoned"))
    }
}

/// Converts an Ora-owned final path into the portable filename reported to the frontend.
fn final_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(DEFAULT_ZIP_FILE_NAME)
        .to_owned()
}

/// Selects and sanitizes a ZIP name from WebView metadata, including unnamed Blob downloads.
fn zip_file_name(url: &Url, suggested_destination: &Path) -> Option<String> {
    let suggested = suggested_destination.file_name().and_then(OsStr::to_str);
    let url_name = url.path_segments().and_then(Iterator::last);
    let candidate = [suggested, url_name]
        .into_iter()
        .flatten()
        .find(|value| has_zip_extension(value));

    candidate
        .map(sanitize_zip_file_name)
        .or_else(|| (url.scheme() == "blob").then(|| DEFAULT_ZIP_FILE_NAME.to_owned()))
}

/// Produces a portable basename while preserving the required lowercase ZIP extension.
fn sanitize_zip_file_name(value: &str) -> String {
    let basename = Path::new(value)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let stem = Path::new(basename)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(basename);
    let mut sanitized = String::new();

    for character in stem.trim_matches([' ', '.']).chars() {
        let character = if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
            '_'
        } else {
            character
        };
        if sanitized.len() + character.len_utf8() > MAX_FILE_STEM_BYTES {
            break;
        }
        sanitized.push(character);
    }

    let sanitized = sanitized.trim_matches([' ', '.']);
    let stem = if sanitized.is_empty() {
        "skill".to_owned()
    } else if is_windows_reserved_name(sanitized) {
        format!("_{sanitized}")
    } else {
        sanitized.to_owned()
    };
    format!("{stem}.zip")
}

/// Recognizes a ZIP extension without accepting names that only contain `.zip` in the middle.
fn has_zip_extension(value: &str) -> bool {
    Path::new(value)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

/// Detects Windows device basenames so saved archives stay portable across supported hosts.
fn is_windows_reserved_name(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

/// Finds the first final and partial path not occupied on disk or by another active download.
fn available_paths(
    directory: &Path,
    file_name: &str,
    active: &HashMap<String, DownloadPaths>,
) -> DownloadPaths {
    let stem = file_name.strip_suffix(".zip").unwrap_or("skill");
    for index in 0.. {
        let candidate = if index == 0 {
            file_name.to_owned()
        } else {
            format!("{stem}-{index}.zip")
        };
        let final_path = directory.join(&candidate);
        let mut part_name = OsString::from(candidate);
        part_name.push(".part");
        let part_path = directory.join(part_name);
        let reserved = active.values().any(|paths| {
            paths_conflict(&paths.final_path, &final_path)
                || paths_conflict(&paths.part_path, &part_path)
        });
        if !reserved && !final_path.exists() && !part_path.exists() {
            return DownloadPaths {
                final_path,
                part_path,
            };
        }
    }
    unreachable!("usize exhaustion prevents allocating another download filename")
}

/// Treats case-only filename differences as conflicts for portable active reservations.
fn paths_conflict(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .file_name()
            .and_then(OsStr::to_str)
            .zip(right.file_name().and_then(OsStr::to_str))
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

/// Removes a partial file while treating an already-removed path as successful cleanup.
fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;
    use tauri::Url;
    use tempfile::TempDir;

    use super::{
        DOWNLOAD_DIRECTORY_NAME, DownloadAcceptance, DownloadFinish, DownloadStatus,
        SkillDownloadCoordinator, sanitize_zip_file_name,
    };

    /// Verifies the coordinator creates its persistent directory below application data.
    #[test]
    fn creates_the_application_download_directory() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let coordinator = SkillDownloadCoordinator::new(temporary.path())
            .expect("create marketplace download coordinator");

        assert_eq!(
            (
                coordinator.directory().to_path_buf(),
                coordinator.directory().is_dir(),
            ),
            (temporary.path().join(DOWNLOAD_DIRECTORY_NAME), true),
        );
    }

    /// Verifies path components, controls, illegal characters, empty names, and devices are safe.
    #[test]
    fn sanitizes_untrusted_zip_file_names() {
        // Windows Path::file_name treats backslashes as separators; Unix keeps them in the stem.
        let windows_drive_path = if cfg!(windows) {
            "evil.zip"
        } else {
            "C__nested_evil.zip"
        };
        assert_eq!(
            [
                "",
                "skill.zip",
                "Mixed.Zip",
                "../nested/unsafe?name.ZIP",
                "C:\\nested\\evil.zip",
                "bad\u{7}name.zip",
                "...zip",
                "CON.zip",
                "a<b>c:d.zip",
            ]
            .map(sanitize_zip_file_name),
            [
                "skill.zip",
                "skill.zip",
                "Mixed.zip",
                "unsafe_name.zip",
                windows_drive_path,
                "bad_name.zip",
                "skill.zip",
                "_CON.zip",
                "a_b_c_d.zip",
            ],
        );
    }

    /// Verifies regular non-ZIP files are rejected and unnamed Blob ZIPs receive a safe fallback.
    #[test]
    fn accepts_only_zip_and_blob_download_requests() {
        let (_temporary, coordinator) = coordinator();
        let mut text_destination = PathBuf::from("readme.txt");
        let mut blob_destination = PathBuf::new();
        let blob_url = url("blob:https://www.skillhub.cn/949b");
        let mut duplicate_destination = PathBuf::from("duplicate.zip");

        assert_eq!(
            (
                coordinator
                    .request(
                        &url("https://www.skillhub.cn/readme.txt"),
                        &mut text_destination,
                    )
                    .expect("reject non-ZIP download"),
                coordinator
                    .request(&blob_url, &mut blob_destination)
                    .expect("accept unnamed Blob download"),
                coordinator
                    .request(&blob_url, &mut duplicate_destination)
                    .expect("reject duplicate active URL"),
                blob_destination
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned),
            ),
            (
                DownloadAcceptance::Rejected,
                DownloadAcceptance::Accepted {
                    file_name: "skill.zip".to_owned(),
                },
                DownloadAcceptance::Rejected,
                Some("skill.zip.part".to_owned()),
            ),
        );
    }

    /// Verifies existing ZIP and partial files survive while a free numeric suffix is selected.
    #[test]
    fn preserves_existing_files_when_reserving_conflicting_names() {
        let (_temporary, coordinator) = coordinator();
        let existing_path = coordinator.directory().join("skill.zip");
        let partial_path = coordinator.directory().join("skill-1.zip.part");
        fs::write(&existing_path, b"existing").expect("write existing ZIP");
        fs::write(&partial_path, b"partial").expect("write existing partial file");
        let mut destination = PathBuf::from("skill.zip");

        let acceptance = coordinator
            .request(&url("https://www.skillhub.cn/skill.zip"), &mut destination)
            .expect("reserve conflicting ZIP name");

        assert_eq!(
            (
                acceptance,
                destination,
                fs::read(existing_path).expect("read existing ZIP"),
                fs::read(partial_path).expect("read existing partial file"),
            ),
            (
                DownloadAcceptance::Accepted {
                    file_name: "skill-2.zip".to_owned(),
                },
                coordinator.directory().join("skill-2.zip.part"),
                b"existing".to_vec(),
                b"partial".to_vec(),
            ),
        );
    }

    /// Verifies a successful download atomically leaves a ZIP and no partial file.
    #[test]
    fn renames_a_successful_partial_download() {
        let (_temporary, coordinator) = coordinator();
        let download_url = url("https://www.skillhub.cn/skill.zip");
        let mut destination = PathBuf::from("skill.zip");
        coordinator
            .request(&download_url, &mut destination)
            .expect("reserve ZIP download");
        fs::write(&destination, b"zip bytes").expect("write partial download");

        let finish = coordinator
            .finish(&download_url, DownloadStatus::Succeeded)
            .expect("finish ZIP download");

        let final_path = coordinator.directory().join("skill.zip");
        assert_eq!(
            (
                finish,
                destination.exists(),
                fs::read(&final_path).expect("read completed ZIP"),
            ),
            (
                DownloadFinish::Completed {
                    file_name: "skill.zip".to_owned(),
                    path: final_path,
                },
                false,
                b"zip bytes".to_vec(),
            ),
        );
    }

    /// Verifies failed or cancelled downloads remove only their own temporary file.
    #[test]
    fn cleans_up_a_failed_partial_download() {
        let (_temporary, coordinator) = coordinator();
        let download_url = url("https://www.skillhub.cn/failing.zip");
        let mut destination = PathBuf::from("failing.zip");
        coordinator
            .request(&download_url, &mut destination)
            .expect("reserve failing download");
        fs::write(&destination, b"partial").expect("write partial download");
        let existing_path = coordinator.directory().join("existing.zip");
        fs::write(&existing_path, b"existing").expect("write existing ZIP");

        let finish = coordinator
            .finish(&download_url, DownloadStatus::Failed)
            .expect("clean failed download");

        assert_eq!(
            (
                finish,
                destination.exists(),
                fs::read(existing_path).expect("read existing ZIP"),
            ),
            (
                DownloadFinish::Failed {
                    file_name: "failing.zip".to_owned(),
                },
                false,
                b"existing".to_vec(),
            ),
        );
    }

    /// Verifies interleaved same-name downloads finish independently without sharing paths.
    #[test]
    fn keeps_parallel_download_states_independent() {
        let (_temporary, coordinator) = coordinator();
        let first_url = url("https://www.skillhub.cn/one/skill.zip");
        let second_url = url("https://www.skillhub.cn/two/skill.zip");
        let mut first_destination = PathBuf::from("skill.zip");
        let mut second_destination = PathBuf::from("SKILL.ZIP");
        coordinator
            .request(&first_url, &mut first_destination)
            .expect("reserve first download");
        coordinator
            .request(&second_url, &mut second_destination)
            .expect("reserve second download");
        fs::write(&first_destination, b"first").expect("write first partial download");
        fs::write(&second_destination, b"second").expect("write second partial download");

        let second_finish = coordinator
            .finish(&second_url, DownloadStatus::Succeeded)
            .expect("finish second download");
        let first_finish = coordinator
            .finish(&first_url, DownloadStatus::Succeeded)
            .expect("finish first download");

        assert_eq!(
            (first_finish, second_finish),
            (
                DownloadFinish::Completed {
                    file_name: "skill.zip".to_owned(),
                    path: coordinator.directory().join("skill.zip"),
                },
                DownloadFinish::Completed {
                    file_name: "SKILL-1.zip".to_owned(),
                    path: coordinator.directory().join("SKILL-1.zip"),
                },
            ),
        );
    }

    /// Verifies a final-name collision created during transfer cannot overwrite an existing ZIP.
    #[test]
    fn avoids_overwriting_a_file_created_during_download() {
        let (_temporary, coordinator) = coordinator();
        let download_url = url("https://www.skillhub.cn/skill.zip");
        let mut destination = PathBuf::from("skill.zip");
        coordinator
            .request(&download_url, &mut destination)
            .expect("reserve ZIP download");
        fs::write(&destination, b"new").expect("write partial download");
        fs::write(coordinator.directory().join("skill.zip"), b"existing")
            .expect("write late conflict");

        let finish = coordinator
            .finish(&download_url, DownloadStatus::Succeeded)
            .expect("finish conflicted download");

        assert_eq!(
            (
                finish,
                fs::read(coordinator.directory().join("skill.zip")).expect("read existing ZIP"),
                fs::read(coordinator.directory().join("skill-1.zip")).expect("read completed ZIP"),
            ),
            (
                DownloadFinish::Completed {
                    file_name: "skill-1.zip".to_owned(),
                    path: coordinator.directory().join("skill-1.zip"),
                },
                b"existing".to_vec(),
                b"new".to_vec(),
            ),
        );
    }

    /// Builds a coordinator while keeping its temporary application directory alive.
    fn coordinator() -> (TempDir, SkillDownloadCoordinator) {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let coordinator = SkillDownloadCoordinator::new(temporary.path())
            .expect("create marketplace download coordinator");
        (temporary, coordinator)
    }

    /// Parses a test URL with a failure message that preserves the invalid fixture.
    fn url(value: &str) -> Url {
        Url::parse(value).unwrap_or_else(|error| panic!("parse test URL {value}: {error}"))
    }
}
