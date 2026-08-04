use crate::error::PrepareError;
use crate::path::{PathValidationError, RelativePath};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// One ordinary file materialized inside a validated source snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFile {
    /// Validated skill-relative path used for boundary scanning and staging.
    pub relative_path: RelativePath,
    /// Number of ordinary bytes stored for this file.
    pub size: u64,
}

/// A validated, materialized tree of one skill source inside OS temporary storage.
///
/// The snapshot never touches the formal skill directory; it only backs preview and
/// per-skill staging during commit.
#[derive(Debug)]
pub struct Snapshot {
    root: PathBuf,
    files: Vec<SnapshotFile>,
}

impl Snapshot {
    /// Returns the absolute destination root that materialized this snapshot.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns every ordinary file sorted by validated relative path.
    pub fn files(&self) -> &[SnapshotFile] {
        &self.files
    }

    /// Reads one file's bytes from the snapshot, resolving it under the root.
    pub fn read_file(&self, relative_path: &RelativePath) -> Result<Vec<u8>, PrepareError> {
        let path = relative_path.to_path(&self.root);
        fs::read(&path).map_err(|error| PrepareError::Io {
            message: format!("failed to read {relative_path}: {error}"),
        })
    }

    /// Returns the snapshot file matching an exact relative path, if present.
    pub fn find_file(&self, relative_path: &RelativePath) -> Option<&SnapshotFile> {
        self.files
            .iter()
            .find(|file| &file.relative_path == relative_path)
    }
}

/// Streams archive and folder entries into a validated snapshot with cumulative limits.
///
/// Every path is validated before anything is written, and the writer tracks entry count,
/// cumulative bytes, and the archive expansion budget so resource-exhaustion attacks abort
/// the whole snapshot instead of a single skill.
#[derive(Debug)]
pub struct SnapshotWriter {
    root: PathBuf,
    limits: crate::limits::Limits,
    byte_cap: u64,
    archive_mode: bool,
    files: Vec<SnapshotFile>,
    seen_keys: HashSet<String>,
    seen_dirs_exact: HashSet<String>,
    seen_dir_keys: HashSet<String>,
    entry_count: usize,
    total_bytes: u64,
}

impl SnapshotWriter {
    /// Creates a writer materializing files under `root`.
    ///
    /// `byte_cap` is the cumulative ordinary-byte budget for the whole source: the flat
    /// snapshot cap for folders, or the expansion-ratio budget for archives.
    pub fn new(
        root: PathBuf,
        limits: crate::limits::Limits,
        byte_cap: u64,
        archive_mode: bool,
    ) -> Result<Self, PrepareError> {
        fs::create_dir_all(&root).map_err(|error| PrepareError::Io {
            message: format!("failed to create snapshot root {}: {error}", root.display()),
        })?;
        Ok(Self {
            root,
            limits,
            byte_cap,
            archive_mode,
            files: Vec::new(),
            seen_keys: HashSet::new(),
            seen_dirs_exact: HashSet::new(),
            seen_dir_keys: HashSet::new(),
            entry_count: 0,
            total_bytes: 0,
        })
    }

    /// Records one explicit directory entry (only metadata; empty dirs are never created).
    pub fn add_directory(&mut self, raw: &str) -> Result<(), PrepareError> {
        self.count_entry()?;
        let trimmed = raw.trim_end_matches('/');
        let path = RelativePath::parse(trimmed).map_err(map_path_error)?;
        let key = path.fold_case_key();
        if self.seen_keys.contains(&key) {
            return Err(PrepareError::ArchivePathCaseConflict);
        }
        self.record_ancestors(&path)?;
        let exact = path.as_str().to_string();
        if self.seen_dirs_exact.contains(&exact) {
            return Ok(());
        }
        if self.seen_dir_keys.contains(&key) {
            return Err(PrepareError::ArchivePathCaseConflict);
        }
        self.seen_dirs_exact.insert(exact);
        self.seen_dir_keys.insert(key);
        Ok(())
    }

    /// Streams one ordinary file into the snapshot under its validated relative path.
    pub fn add_file(&mut self, raw: &str, reader: impl Read) -> Result<(), PrepareError> {
        self.count_entry()?;
        let path = RelativePath::parse(raw).map_err(map_path_error)?;
        let key = path.fold_case_key();
        if self.seen_keys.contains(&key) || self.seen_dir_keys.contains(&key) {
            return Err(PrepareError::ArchivePathCaseConflict);
        }
        self.record_ancestors(&path)?;
        self.seen_keys.insert(key);

        let destination = path.to_path(&self.root);
        let parent = destination.parent().ok_or_else(|| PrepareError::Io {
            message: "snapshot file path has no parent".to_string(),
        })?;
        fs::create_dir_all(parent).map_err(|error| PrepareError::Io {
            message: format!(
                "failed to create snapshot directory {}: {error}",
                parent.display()
            ),
        })?;
        let mut file = fs::File::create(&destination).map_err(|error| PrepareError::Io {
            message: format!(
                "failed to create snapshot file {}: {error}",
                destination.display()
            ),
        })?;
        let remaining = self.byte_cap.saturating_sub(self.total_bytes);
        let mut budgeted = BudgetReader::new(reader, remaining);
        let copied =
            io::copy(&mut budgeted, &mut file).map_err(map_copy_error(self.archive_mode))?;
        self.total_bytes += copied;
        self.files.push(SnapshotFile {
            relative_path: path,
            size: copied,
        });
        Ok(())
    }

    /// Finalizes the snapshot, sorting files deterministically for boundary scanning.
    pub fn finish(self) -> Snapshot {
        let mut files = self.files;
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Snapshot {
            root: self.root,
            files,
        }
    }

    /// Increments the shared entry counter and rejects entry-count exhaustion.
    fn count_entry(&mut self) -> Result<(), PrepareError> {
        self.entry_count += 1;
        if self.entry_count > self.limits.max_entries {
            return Err(PrepareError::TooManyEntries {
                max_entries: self.limits.max_entries,
            });
        }
        Ok(())
    }

    /// Registers every ancestor directory of a path, rejecting file/type and case conflicts.
    fn record_ancestors(&mut self, path: &RelativePath) -> Result<(), PrepareError> {
        let mut ancestor = path.parent();
        while let Some(directory) = ancestor {
            let exact = directory.as_str().to_string();
            if !self.seen_dirs_exact.contains(&exact) {
                let directory_key = directory.fold_case_key();
                if self.seen_dir_keys.contains(&directory_key)
                    || self.seen_keys.contains(&directory_key)
                {
                    return Err(PrepareError::ArchivePathCaseConflict);
                }
                self.seen_dirs_exact.insert(exact);
                self.seen_dir_keys.insert(directory_key);
            }
            ancestor = directory.parent();
        }
        Ok(())
    }
}

/// Wraps one entry reader with the remaining snapshot byte budget.
struct BudgetReader<R> {
    inner: R,
    remaining: u64,
}

impl<R> BudgetReader<R> {
    fn new(inner: R, remaining: u64) -> Self {
        Self { inner, remaining }
    }
}

impl<R: Read> Read for BudgetReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if read as u64 > self.remaining {
            return Err(io::Error::other(BudgetExceeded));
        }
        self.remaining -= read as u64;
        Ok(read)
    }
}

/// Marker carried inside the copy error that distinguishes budget exhaustion from I/O failure.
#[derive(Debug)]
struct BudgetExceeded;

impl std::fmt::Display for BudgetExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("snapshot byte budget exceeded")
    }
}

impl std::error::Error for BudgetExceeded {}

/// Classifies copy failures into stable expansion or capacity errors.
fn map_copy_error(archive_mode: bool) -> impl FnOnce(io::Error) -> PrepareError {
    move |error| {
        if error
            .get_ref()
            .is_some_and(<dyn std::error::Error + Send + Sync>::is::<BudgetExceeded>)
        {
            if archive_mode {
                PrepareError::ArchiveExpansionRatioExceeded
            } else {
                PrepareError::TotalBytesExceeded
            }
        } else {
            PrepareError::Io {
                message: format!("failed to copy source bytes: {error}"),
            }
        }
    }
}

/// Maps relative-path validation failures to stable whole-session error codes.
fn map_path_error(error: PathValidationError) -> PrepareError {
    match error {
        PathValidationError::EncodingInvalid => PrepareError::ArchivePathEncodingInvalid,
        PathValidationError::Unsafe => PrepareError::UnsafePath,
        PathValidationError::SegmentTooLong => PrepareError::PathSegmentTooLong,
        PathValidationError::TooLong => PrepareError::PathTooLong,
        PathValidationError::TooDeep => PrepareError::PathTooDeep,
    }
}
