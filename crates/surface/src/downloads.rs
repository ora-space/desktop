use crate::ids::WebviewLabel;
use ora_utils::fs::{next_available_file_name, sanitize_file_name};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use time::OffsetDateTime;
use url::Url;

/// Stem used when neither the suggested destination nor the URL yields a usable name.
const FALLBACK_STEM: &str = "download";
/// Suffix of in-flight files; completed files never carry it.
const PART_SUFFIX: &str = ".part";

/// Supplies the local instants stamped on reservations and completed downloads.
///
/// Injected so the coordinator can be unit-tested with a fixed instant while production code
/// reads Ora's process-wide local clock. Implementations must return local (not UTC) time.
pub trait DownloadClock {
    /// Returns the current local time.
    fn now_local(&self) -> OffsetDateTime;
}

/// Production clock backed by the process-wide timezone configured at logging startup.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalDownloadClock;

impl DownloadClock for LocalDownloadClock {
    fn now_local(&self) -> OffsetDateTime {
        ora_logging::clock::now_local()
    }
}

/// Coordinates collision-free temporary and final paths for downloads started by surfaces.
///
/// Reservations are keyed by `(label, url)` so two surfaces downloading the same URL, or one
/// surface downloading two equally named files, never share a path.
#[derive(Debug, Default)]
pub struct DownloadCoordinator<C: DownloadClock = LocalDownloadClock> {
    clock: C,
    active: Mutex<HashMap<DownloadKey, Reservation>>,
    next_id: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DownloadKey {
    label: WebviewLabel,
    url: String,
}

/// Identifies one download within the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DownloadId(u64);

impl DownloadId {
    /// Wraps a raw counter value; exposed so hosts can build fixtures and round-trip ids.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw counter value for events and logs.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// One in-flight download and the paths reserved for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub id: DownloadId,
    pub part_path: PathBuf,
    pub final_path: PathBuf,
    pub file_name: String,
    pub page_url: Option<Url>,
    pub started_at: OffsetDateTime,
}

/// Decision for a download request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadAcceptance {
    Accepted {
        id: DownloadId,
        file_name: String,
        part_path: PathBuf,
    },
    Rejected(RejectReason),
}

/// Why a download request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// The same surface is already downloading this URL.
    DuplicateInFlight,
    /// The plugin download directory could not be created.
    DirectoryUnavailable,
}

/// How the browser engine reported the end of a transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadStatus {
    Succeeded,
    Failed,
}

/// Outcome of finishing a download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadFinish {
    /// Boxed because the payload dwarfs the other variants.
    Completed(Box<CompletedDownload>),
    Failed {
        id: DownloadId,
        file_name: String,
    },
    /// No reservation matched; the host only logs this.
    Unknown,
}

/// A download that reached its final path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedDownload {
    pub id: DownloadId,
    pub page_url: Option<Url>,
    pub source_url: Url,
    pub file_name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub completed_at: OffsetDateTime,
}

impl<C: DownloadClock> DownloadCoordinator<C> {
    /// Creates a coordinator with no reservations that stamps times from `clock`.
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        }
    }

    /// Reserves a unique `.part` destination inside `directory`, the plugin's download folder.
    ///
    /// The directory is created here rather than at construction because it can disappear while
    /// Ora runs and the request is the last responsible moment to recreate it.
    pub fn request(
        &self,
        label: &WebviewLabel,
        directory: &Path,
        source_url: &Url,
        page_url: Option<Url>,
        suggested_destination: &Path,
    ) -> io::Result<DownloadAcceptance> {
        if fs::create_dir_all(directory).is_err() {
            return Ok(DownloadAcceptance::Rejected(
                RejectReason::DirectoryUnavailable,
            ));
        }
        let file_name = candidate_file_name(source_url, suggested_destination);
        let mut active = self.lock_active()?;
        let key = DownloadKey {
            label: label.clone(),
            url: source_url.to_string(),
        };
        if active.contains_key(&key) {
            return Ok(DownloadAcceptance::Rejected(
                RejectReason::DuplicateInFlight,
            ));
        }
        let final_path = next_available_file_name(directory, &file_name, |candidate| {
            reserved_or_partial(active.values(), candidate)
        });
        let reservation = Reservation {
            id: DownloadId(self.next_id.fetch_add(1, Ordering::Relaxed)),
            part_path: part_path_of(&final_path),
            file_name: file_name_of(&final_path),
            final_path,
            page_url,
            started_at: self.clock.now_local(),
        };
        let acceptance = DownloadAcceptance::Accepted {
            id: reservation.id,
            file_name: reservation.file_name.clone(),
            part_path: reservation.part_path.clone(),
        };
        active.insert(key, reservation);
        Ok(acceptance)
    }

    /// Promotes a successful `.part` file to its final name or removes it after failure.
    pub fn finish(
        &self,
        label: &WebviewLabel,
        source_url: &Url,
        status: DownloadStatus,
    ) -> io::Result<DownloadFinish> {
        let Some(reservation) = self.take(label, source_url, status)? else {
            return Ok(DownloadFinish::Unknown);
        };
        match status {
            DownloadStatus::Failed => {
                remove_file_if_present(&reservation.part_path)?;
                Ok(DownloadFinish::Failed {
                    id: reservation.id,
                    file_name: reservation.file_name,
                })
            }
            DownloadStatus::Succeeded => {
                if let Err(error) = fs::rename(&reservation.part_path, &reservation.final_path) {
                    // A failed handoff must not leave a partial file that looks complete later.
                    let _ = remove_file_if_present(&reservation.part_path);
                    return Err(error);
                }
                let size_bytes = fs::metadata(&reservation.final_path)?.len();
                Ok(DownloadFinish::Completed(Box::new(CompletedDownload {
                    id: reservation.id,
                    page_url: reservation.page_url,
                    source_url: source_url.clone(),
                    file_name: reservation.file_name,
                    path: reservation.final_path,
                    size_bytes,
                    completed_at: self.clock.now_local(),
                })))
            }
        }
    }

    /// Removes the reservation and, before a successful promotion, re-picks the final name if
    /// another process created it during the transfer so the foreign file is never replaced.
    fn take(
        &self,
        label: &WebviewLabel,
        source_url: &Url,
        status: DownloadStatus,
    ) -> io::Result<Option<Reservation>> {
        let mut active = self.lock_active()?;
        let key = DownloadKey {
            label: label.clone(),
            url: source_url.to_string(),
        };
        let Some(mut reservation) = active.remove(&key) else {
            return Ok(None);
        };
        if status == DownloadStatus::Succeeded && reservation.final_path.exists() {
            let directory = reservation
                .final_path
                .parent()
                .map_or_else(PathBuf::new, Path::to_path_buf);
            reservation.final_path =
                next_available_file_name(&directory, &reservation.file_name, |candidate| {
                    reserved_or_partial(active.values(), candidate)
                });
            reservation.file_name = file_name_of(&reservation.final_path);
        }
        Ok(Some(reservation))
    }

    /// Converts a poisoned lock into an I/O error so the host callback can reject safely.
    fn lock_active(
        &self,
    ) -> io::Result<std::sync::MutexGuard<'_, HashMap<DownloadKey, Reservation>>> {
        self.active
            .lock()
            .map_err(|_| io::Error::other("surface download state lock is poisoned"))
    }
}

/// Picks the sanitized name: suggested destination, then the last URL path segment, then the
/// fallback stem (for example `blob:` URLs carry neither).
fn candidate_file_name(source_url: &Url, suggested_destination: &Path) -> String {
    let suggested = suggested_destination
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|name| !name.is_empty());
    let url_name = source_url
        .path_segments()
        .and_then(Iterator::last)
        .filter(|segment| !segment.is_empty());
    sanitize_file_name(suggested.or(url_name).unwrap_or_default(), FALLBACK_STEM)
}

/// Treats a candidate as taken when a reservation claims it (case-insensitively, so the result
/// is portable to case-insensitive filesystems) or its `.part` twin already exists on disk.
fn reserved_or_partial<'a>(
    reservations: impl Iterator<Item = &'a Reservation>,
    candidate: &Path,
) -> bool {
    let part = part_path_of(candidate);
    let mut reservations = reservations;
    reservations.any(|reservation| {
        names_conflict(&reservation.final_path, candidate)
            || names_conflict(&reservation.part_path, &part)
    }) || part.exists()
}

/// Compares basenames ignoring ASCII case.
fn names_conflict(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .file_name()
            .and_then(OsStr::to_str)
            .zip(right.file_name().and_then(OsStr::to_str))
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

/// Derives the in-flight path of a final path.
fn part_path_of(final_path: &Path) -> PathBuf {
    let mut name = final_path
        .file_name()
        .map_or_else(Default::default, OsStr::to_os_string);
    name.push(PART_SUFFIX);
    final_path.with_file_name(name)
}

/// Extracts the portable basename reported to the frontend.
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(FALLBACK_STEM)
        .to_owned()
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
    use super::{
        CompletedDownload, DownloadAcceptance, DownloadClock, DownloadCoordinator, DownloadFinish,
        DownloadId, DownloadStatus, RejectReason,
    };
    use crate::ids::{SurfaceDefinitionId, SurfaceInstanceId, WebviewLabel};
    use ora_domain::PluginId;
    use ora_plugin_manager::SurfaceId;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;
    use time::OffsetDateTime;
    use time::macros::datetime;
    use url::Url;

    /// Reports one fixed local instant so completed downloads compare as whole objects.
    struct FixedClock;

    const NOW: OffsetDateTime = datetime!(2026-08-20 10:30:00 +08:00);

    impl DownloadClock for FixedClock {
        fn now_local(&self) -> OffsetDateTime {
            NOW
        }
    }

    /// Builds a coordinator with the fixed clock.
    fn coordinator() -> DownloadCoordinator<FixedClock> {
        DownloadCoordinator::new(FixedClock)
    }

    /// Builds a label for the given instance number.
    fn label(instance: u64) -> WebviewLabel {
        WebviewLabel::remote(
            &SurfaceDefinitionId {
                plugin_id: PluginId::new("official", "ora-space.skillhub").expect("plugin id"),
                surface_id: SurfaceId::parse("market").expect("valid surface id"),
            },
            SurfaceInstanceId::new(instance),
        )
    }

    /// Parses a test URL with a failure message that preserves the invalid fixture.
    fn url(value: &str) -> Url {
        Url::parse(value).unwrap_or_else(|error| panic!("parse test URL {value}: {error}"))
    }

    /// Requests a download and returns the accepted part path, panicking on rejection.
    fn accept(
        coordinator: &DownloadCoordinator<FixedClock>,
        label: &WebviewLabel,
        directory: &Path,
        source: &Url,
        suggested: &str,
    ) -> (DownloadId, String, PathBuf) {
        match coordinator
            .request(label, directory, source, None, Path::new(suggested))
            .expect("request download")
        {
            DownloadAcceptance::Accepted {
                id,
                file_name,
                part_path,
            } => (id, file_name, part_path),
            DownloadAcceptance::Rejected(reason) => panic!("unexpected rejection {reason:?}"),
        }
    }

    /// Verifies naming precedence (suggestion, URL segment, fallback) and duplicate rejection per
    /// `(label, url)`, while a different label may download the same URL.
    #[test]
    fn names_downloads_and_rejects_duplicates_per_surface() {
        let temporary = TempDir::new().expect("temp dir");
        let coordinator = coordinator();
        let blob = url("blob:https://www.skillhub.cn/949b");
        let tar = url("https://www.skillhub.cn/files/pack.tar.gz");
        let first = coordinator
            .request(
                &label(1),
                temporary.path(),
                &tar,
                None,
                Path::new("My Skill (1).zip"),
            )
            .expect("request");
        let from_url = coordinator.request(&label(1), temporary.path(), &tar, None, Path::new(""));
        let other_surface = coordinator
            .request(&label(2), temporary.path(), &tar, None, Path::new(""))
            .expect("request");
        let fallback = coordinator
            .request(&label(1), temporary.path(), &blob, None, Path::new(""))
            .expect("request");

        assert_eq!(
            (first, from_url.expect("request"), other_surface, fallback),
            (
                DownloadAcceptance::Accepted {
                    id: DownloadId(0),
                    file_name: "My Skill (1).zip".to_owned(),
                    part_path: temporary.path().join("My Skill (1).zip.part"),
                },
                DownloadAcceptance::Rejected(RejectReason::DuplicateInFlight),
                DownloadAcceptance::Accepted {
                    id: DownloadId(1),
                    file_name: "pack.tar.gz".to_owned(),
                    part_path: temporary.path().join("pack.tar.gz.part"),
                },
                DownloadAcceptance::Accepted {
                    id: DownloadId(2),
                    file_name: "download".to_owned(),
                    part_path: temporary.path().join("download.part"),
                },
            )
        );
    }

    /// Verifies an unusable directory is reported as a rejection rather than an error.
    #[test]
    fn rejects_when_directory_cannot_be_created() {
        let temporary = TempDir::new().expect("temp dir");
        let blocker = temporary.path().join("file");
        fs::write(&blocker, b"not a directory").expect("write blocker");
        let coordinator = coordinator();

        let acceptance = coordinator
            .request(
                &label(1),
                &blocker.join("downloads"),
                &url("https://www.skillhub.cn/skill.zip"),
                None,
                Path::new("skill.zip"),
            )
            .expect("request");

        assert_eq!(
            acceptance,
            DownloadAcceptance::Rejected(RejectReason::DirectoryUnavailable)
        );
    }

    /// Verifies existing final and partial files survive while a free numeric suffix is selected,
    /// and that a second reservation of the same name skips the first one's slot.
    #[test]
    fn preserves_existing_files_when_reserving_conflicting_names() {
        let temporary = TempDir::new().expect("temp dir");
        let directory = temporary.path();
        fs::write(directory.join("skill.zip"), b"existing").expect("write existing");
        fs::write(directory.join("skill-1.zip.part"), b"partial").expect("write partial");
        let coordinator = coordinator();

        let (_, first_name, first_part) = accept(
            &coordinator,
            &label(1),
            directory,
            &url("https://www.skillhub.cn/one/skill.zip"),
            "skill.zip",
        );
        let (_, second_name, second_part) = accept(
            &coordinator,
            &label(1),
            directory,
            &url("https://www.skillhub.cn/two/skill.zip"),
            "skill.zip",
        );

        assert_eq!(
            (
                first_name,
                first_part,
                second_name,
                second_part,
                fs::read(directory.join("skill.zip")).expect("read existing"),
            ),
            (
                "skill-2.zip".to_owned(),
                directory.join("skill-2.zip.part"),
                "skill-3.zip".to_owned(),
                directory.join("skill-3.zip.part"),
                b"existing".to_vec(),
            )
        );
    }

    /// Verifies reservations conflict on case-only differences so results stay portable to
    /// case-insensitive filesystems regardless of the host running the test.
    #[test]
    fn treats_case_only_reservation_differences_as_conflicts() {
        let temporary = TempDir::new().expect("temp dir");
        let coordinator = coordinator();

        let (_, first_name, _) = accept(
            &coordinator,
            &label(1),
            temporary.path(),
            &url("https://www.skillhub.cn/one/skill.zip"),
            "Skill.zip",
        );
        let (_, second_name, _) = accept(
            &coordinator,
            &label(2),
            temporary.path(),
            &url("https://www.skillhub.cn/one/skill.zip"),
            "SKILL.ZIP",
        );

        assert_eq!(
            (first_name, second_name),
            ("Skill.zip".to_owned(), "SKILL-1.ZIP".to_owned())
        );
    }

    /// Verifies a successful download is renamed, measured, and reported with its page URL, and
    /// that finishing it again is unknown.
    #[test]
    fn renames_a_successful_partial_download() {
        let temporary = TempDir::new().expect("temp dir");
        let coordinator = coordinator();
        let source = url("https://www.skillhub.cn/skill.zip");
        let page = url("https://www.skillhub.cn/skills/example");
        let DownloadAcceptance::Accepted { part_path, .. } = coordinator
            .request(
                &label(1),
                temporary.path(),
                &source,
                Some(page.clone()),
                Path::new("skill.zip"),
            )
            .expect("request")
        else {
            panic!("expected acceptance");
        };
        fs::write(&part_path, b"zip bytes").expect("write partial");

        let finish = coordinator
            .finish(&label(1), &source, DownloadStatus::Succeeded)
            .expect("finish");
        let again = coordinator
            .finish(&label(1), &source, DownloadStatus::Succeeded)
            .expect("finish again");

        let final_path = temporary.path().join("skill.zip");
        assert_eq!(
            (
                finish,
                part_path.exists(),
                fs::read(&final_path).expect("read final"),
                again,
            ),
            (
                DownloadFinish::Completed(Box::new(CompletedDownload {
                    id: DownloadId(0),
                    page_url: Some(page),
                    source_url: source,
                    file_name: "skill.zip".to_owned(),
                    path: final_path,
                    size_bytes: 9,
                    completed_at: NOW,
                })),
                false,
                b"zip bytes".to_vec(),
                DownloadFinish::Unknown,
            )
        );
    }

    /// Verifies failed or cancelled downloads remove only their own temporary file.
    #[test]
    fn cleans_up_a_failed_partial_download() {
        let temporary = TempDir::new().expect("temp dir");
        let coordinator = coordinator();
        let source = url("https://www.skillhub.cn/failing.zip");
        let (id, _, part_path) = accept(
            &coordinator,
            &label(1),
            temporary.path(),
            &source,
            "failing.zip",
        );
        fs::write(&part_path, b"partial").expect("write partial");
        let existing = temporary.path().join("existing.zip");
        fs::write(&existing, b"existing").expect("write existing");

        let finish = coordinator
            .finish(&label(1), &source, DownloadStatus::Failed)
            .expect("finish");

        assert_eq!(
            (
                finish,
                part_path.exists(),
                fs::read(existing).expect("read existing")
            ),
            (
                DownloadFinish::Failed {
                    id,
                    file_name: "failing.zip".to_owned(),
                },
                false,
                b"existing".to_vec(),
            )
        );
    }

    /// Verifies a final-name collision created during transfer cannot overwrite a foreign file.
    #[test]
    fn avoids_overwriting_a_file_created_during_download() {
        let temporary = TempDir::new().expect("temp dir");
        let coordinator = coordinator();
        let source = url("https://www.skillhub.cn/skill.zip");
        let (_, _, part_path) = accept(
            &coordinator,
            &label(1),
            temporary.path(),
            &source,
            "skill.zip",
        );
        fs::write(&part_path, b"new").expect("write partial");
        fs::write(temporary.path().join("skill.zip"), b"existing").expect("write late conflict");

        let finish = coordinator
            .finish(&label(1), &source, DownloadStatus::Succeeded)
            .expect("finish");

        assert_eq!(
            (
                finish,
                fs::read(temporary.path().join("skill.zip")).expect("read existing"),
                fs::read(temporary.path().join("skill-1.zip")).expect("read completed"),
            ),
            (
                DownloadFinish::Completed(Box::new(CompletedDownload {
                    id: DownloadId(0),
                    page_url: None,
                    source_url: source,
                    file_name: "skill-1.zip".to_owned(),
                    path: temporary.path().join("skill-1.zip"),
                    size_bytes: 3,
                    completed_at: NOW,
                })),
                b"existing".to_vec(),
                b"new".to_vec(),
            )
        );
    }
}
