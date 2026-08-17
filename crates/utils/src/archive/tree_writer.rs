use super::error::ArchiveError;
use super::extracted::{ExtractedFile, ExtractedTree};
use super::limits::ExtractLimits;
use crate::path::StrictRelativePath;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

/// Selects which stable error a cumulative byte-budget overflow maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ByteBudgetKind {
    /// The budget is the archive expansion-ratio allowance.
    ArchiveExpansion,
    /// The budget is the flat total-bytes cap for a copied folder.
    FlatTotal,
}

/// Streams archive and folder entries into a validated tree with cumulative limits.
///
/// Every path is validated before anything is written, and the writer tracks entry count,
/// cumulative bytes, and the byte budget so resource-exhaustion attacks abort the whole tree
/// instead of a single entry.
#[derive(Debug)]
pub(super) struct TreeWriter {
    root: PathBuf,
    limits: ExtractLimits,
    byte_cap: u64,
    budget_kind: ByteBudgetKind,
    files: Vec<ExtractedFile>,
    seen_keys: HashSet<String>,
    seen_dirs_exact: HashSet<String>,
    seen_dir_keys: HashSet<String>,
    entry_count: usize,
    total_bytes: u64,
}

impl TreeWriter {
    /// Creates a writer materializing files under `root`.
    ///
    /// `byte_cap` is the cumulative ordinary-byte budget for the whole tree: the flat total cap
    /// for folders, or the expansion-ratio budget for archives.
    pub(super) fn new(
        root: PathBuf,
        limits: ExtractLimits,
        byte_cap: u64,
        budget_kind: ByteBudgetKind,
    ) -> Result<Self, ArchiveError> {
        fs::create_dir_all(&root).map_err(|error| ArchiveError::Io {
            message: format!("failed to create tree root {}: {error}", root.display()),
        })?;
        Ok(Self {
            root,
            limits,
            byte_cap,
            budget_kind,
            files: Vec::new(),
            seen_keys: HashSet::new(),
            seen_dirs_exact: HashSet::new(),
            seen_dir_keys: HashSet::new(),
            entry_count: 0,
            total_bytes: 0,
        })
    }

    /// Records one explicit directory entry (only metadata; empty dirs are never created).
    pub(super) fn add_directory(&mut self, raw: &str) -> Result<(), ArchiveError> {
        self.count_entry()?;
        let trimmed = raw.trim_end_matches('/');
        let path = StrictRelativePath::parse_with_limits(trimmed, &self.limits.path)
            .map_err(ArchiveError::Path)?;
        let key = path.fold_case_key();
        if self.seen_keys.contains(&key) {
            return Err(ArchiveError::PathCaseConflict);
        }
        self.record_ancestors(&path)?;
        let exact = path.as_str().to_string();
        if self.seen_dirs_exact.contains(&exact) {
            return Ok(());
        }
        if self.seen_dir_keys.contains(&key) {
            return Err(ArchiveError::PathCaseConflict);
        }
        self.seen_dirs_exact.insert(exact);
        self.seen_dir_keys.insert(key);
        Ok(())
    }

    /// Streams one ordinary file into the tree under its validated relative path.
    pub(super) fn add_file(&mut self, raw: &str, reader: impl Read) -> Result<(), ArchiveError> {
        self.count_entry()?;
        let path = StrictRelativePath::parse_with_limits(raw, &self.limits.path)
            .map_err(ArchiveError::Path)?;
        let key = path.fold_case_key();
        if self.seen_keys.contains(&key) || self.seen_dir_keys.contains(&key) {
            return Err(ArchiveError::PathCaseConflict);
        }
        self.record_ancestors(&path)?;
        self.seen_keys.insert(key);

        let destination = path.to_path(&self.root);
        let parent = destination.parent().ok_or_else(|| ArchiveError::Io {
            message: "tree file path has no parent".to_string(),
        })?;
        fs::create_dir_all(parent).map_err(|error| ArchiveError::Io {
            message: format!(
                "failed to create tree directory {}: {error}",
                parent.display()
            ),
        })?;
        let mut file = fs::File::create(&destination).map_err(|error| ArchiveError::Io {
            message: format!(
                "failed to create tree file {}: {error}",
                destination.display()
            ),
        })?;
        let remaining = self.byte_cap.saturating_sub(self.total_bytes);
        let mut budgeted = BudgetReader::new(reader, remaining);
        let copied =
            io::copy(&mut budgeted, &mut file).map_err(map_copy_error(self.budget_kind))?;
        self.total_bytes += copied;
        self.files.push(ExtractedFile {
            relative_path: path,
            size: copied,
        });
        Ok(())
    }

    /// Finalizes the tree listing.
    pub(super) fn finish(self) -> ExtractedTree {
        ExtractedTree::new(self.root, self.files)
    }

    /// Increments the shared entry counter and rejects entry-count exhaustion.
    fn count_entry(&mut self) -> Result<(), ArchiveError> {
        self.entry_count += 1;
        if self.entry_count > self.limits.max_entries {
            return Err(ArchiveError::TooManyEntries {
                max_entries: self.limits.max_entries,
            });
        }
        Ok(())
    }

    /// Registers every ancestor directory of a path, rejecting file/type and case conflicts.
    fn record_ancestors(&mut self, path: &StrictRelativePath) -> Result<(), ArchiveError> {
        let mut ancestor = path.parent();
        while let Some(directory) = ancestor {
            let exact = directory.as_str().to_string();
            if !self.seen_dirs_exact.contains(&exact) {
                let directory_key = directory.fold_case_key();
                if self.seen_dir_keys.contains(&directory_key)
                    || self.seen_keys.contains(&directory_key)
                {
                    return Err(ArchiveError::PathCaseConflict);
                }
                self.seen_dirs_exact.insert(exact);
                self.seen_dir_keys.insert(directory_key);
            }
            ancestor = directory.parent();
        }
        Ok(())
    }
}

/// Wraps one entry reader with the remaining tree byte budget.
///
/// Unlike [`Read::take`], exceeding the budget is an error rather than silent truncation, so a
/// budget overflow aborts the whole tree instead of storing a partial file.
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
        formatter.write_str("tree byte budget exceeded")
    }
}

impl std::error::Error for BudgetExceeded {}

/// Classifies copy failures into stable expansion or capacity errors.
fn map_copy_error(budget_kind: ByteBudgetKind) -> impl FnOnce(io::Error) -> ArchiveError {
    move |error| {
        if error
            .get_ref()
            .is_some_and(<dyn std::error::Error + Send + Sync>::is::<BudgetExceeded>)
        {
            match budget_kind {
                ByteBudgetKind::ArchiveExpansion => ArchiveError::ExpansionRatioExceeded,
                ByteBudgetKind::FlatTotal => ArchiveError::TotalBytesExceeded,
            }
        } else {
            ArchiveError::Io {
                message: format!("failed to copy source bytes: {error}"),
            }
        }
    }
}
